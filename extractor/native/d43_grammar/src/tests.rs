use alloc::vec::Vec;
use alloc::{format, vec};

use crate::encode;
use crate::*;

const TEST_LIMITS: Limits = Limits::new(64, 64);

fn test_decode_context(
    accepted_families: crate::decode::D43AcceptedFamilies,
) -> crate::decode::DecodeContext {
    crate::decode::DecodeContext::new(
        accepted_families,
        64,
        64,
        i32::MAX as u32,
        62,
        i32::MAX as u32,
        i32::MAX as u32,
        1_000_000,
        MAX_AMOUNT_RAW,
    )
}

#[test]
fn eager_decoder_accepts_the_minimal_payload() {
    let payload = core_bytes(&[]).done();
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(report.structural_stop, None);
    assert_eq!(crate::decode::select_first_fatal(&report), None);
    assert_eq!(report.document.consumed, payload.len());
    assert_eq!(report.units.market_trade_contract_rows, Some(0));
}

#[test]
fn eager_decoder_walks_every_declared_block_of_every_family() {
    for trailer in Trailer::ALL {
        let payload = a_fixture_with_every_block_filled(trailer);
        let report = crate::decode::decode_record(
            &payload,
            test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
        );
        assert_eq!(report.structural_stop, None, "{}", trailer.label());
        match trailer {
            Trailer::Detail => assert!(!report.document.detail_rows.is_empty()),
            Trailer::ResourceFlow => assert!(!report.document.flow_rows.is_empty()),
            Trailer::FactoryHeader => assert!(!report.document.factory_rows.is_empty()),
            Trailer::ResourceRow => {
                assert!(!report.document.resource_identities.is_empty());
                assert!(!report.document.resource_rows.is_empty());
            }
            Trailer::ClassicDeal => {
                assert!(!report.document.deal_witnesses.is_empty());
                assert!(!report.document.deal_incomplete_owners.is_empty());
                assert!(!report.document.deal_rows.is_empty());
            }
            Trailer::Activation => assert!(report.document.activation_header.is_some()),
            Trailer::Stockpile => {
                assert!(!report.document.stockpile_countries.is_empty());
                assert!(!report.document.stockpile_items.is_empty());
            }
            Trailer::Residual => assert!(!report.document.residual_rows.is_empty()),
            Trailer::LendLease => assert!(!report.document.lend_lease_rows.is_empty()),
        }
    }
}

#[test]
fn semantic_family_findings_do_not_cut_structurally_read_units() {
    let append_resource = |builder: &mut Bytes| {
        mtrr(
            builder,
            ResourceRowVersion::Identity,
            2,
            &["oil", "steel"],
            &[21],
            &[
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
            ],
        );
    };

    let mut builder = core_bytes(&[]);
    mts1_v1(&mut builder);
    append_resource(&mut builder);
    let mut payload = builder.done();
    let clean = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let resource_row = clean.document.resource_rows[0].at;
    payload[resource_row + 56..resource_row + 60].copy_from_slice(&0u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(report.units.market_trade_stockpile_country_rows, Some(1));
    assert_eq!(report.units.market_trade_resource_rows, Some(2));

    let mut builder = core_bytes(&[]);
    mts1_v1(&mut builder);
    mtrf(
        &mut builder,
        FlowVersion::Fixed7,
        FIXED7_RESOURCE_COUNT,
        &[1, 2, 3, 4, 5, 6],
    );
    let mut payload = builder.done();
    let clean = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let item = clean.document.stockpile_items[0].at;
    payload[item + 4..item + 8].copy_from_slice(&9u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(report.units.market_trade_resource_flow_rows, Some(6));
    assert_eq!(report.units.market_trade_stockpile_item_rows, Some(2));
}

#[test]
fn decoded_identity_text_is_self_contained() {
    let mut payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let identity = &report.document.resource_identities[0];
    assert_eq!(crate::decode::identity_token(identity), Some("oil"));
    let token = identity.token.clone().expect("wire provenance");
    payload[token].fill(b'x');
    assert_eq!(crate::decode::identity_token(identity), Some("oil"));
}

#[test]
fn structural_stop_retains_an_earlier_semantic_finding() {
    let mut payload = core_bytes(&[[1, 1 << 31, 0]]).done();
    payload.extend_from_slice(b"MT");
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report.structural_stop.is_some());
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CoreUnknownFlags));
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43StructuralViolation));
}

#[test]
fn multi_defect_core_retains_all_and_selects_the_historical_first() {
    let mut payload = core_bytes(&[[0, 1 << 31, 0]]).done();
    payload[CORE_HEADER_LEN + 44..CORE_HEADER_LEN + 48].copy_from_slice(&9u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CoreContractIdZero));
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CoreUnknownFlags));
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CoreReserved));
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|f| f.rule),
        Some(crate::decode::D43DecodeRule::D43CoreContractIdZero)
    );
    assert_eq!(
        crate::decode::select_first_fatal_through(
            &report,
            crate::decode::D43VerdictBoundary::D43TrailersAndCrossFamily,
        ),
        None
    );
}

#[test]
fn fatal_selection_preserves_row_order_before_rule_order() {
    let payload = core_bytes(&[[1, 1 << 31, 0], [0, 0, 0]]).done();
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43CoreUnknownFlags)
    );
}

#[test]
fn fatal_selection_preserves_mtrr_block_order() {
    let mut payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(cursor) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *cursor.header();
    let identity_one = header.identity_at + header.identity_stride;
    payload[identity_one..identity_one + 2].copy_from_slice(&0u16.to_le_bytes());
    payload[header.rows_at..header.rows_at + 4].copy_from_slice(&0u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43IdentityLength)
    );
}

#[test]
fn fatal_selection_preserves_mtrd_block_order() {
    let mut payload = a_fixture_with_every_block_filled(Trailer::ClassicDeal);
    let TrailerBody::ClassicDeal(cursor) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let header = *cursor.header();
    payload[header.witnesses_at + 4..header.witnesses_at + 8].copy_from_slice(&0u32.to_le_bytes());
    payload[header.rows_at..header.rows_at + 4].copy_from_slice(&0u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43DealWitnessRange)
    );
}

#[test]
fn fatal_selection_preserves_mts1_block_order() {
    let mut payload = a_fixture_with_every_block_filled(Trailer::Stockpile);
    let TrailerBody::Stockpile(cursor) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let header = *cursor.header();
    payload[header.countries_at..header.countries_at + 4].copy_from_slice(&0u32.to_le_bytes());
    payload[header.items_at..header.items_at + 4].copy_from_slice(&0u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43StockpileCountryTagRange)
    );
}

#[test]
fn mts1_country_coverage_precedes_item_validation() {
    let mut builder = core_bytes(&[]);
    mts1_v1(&mut builder);
    let mut payload = builder.done();
    let TrailerBody::Stockpile(cursor) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let header = *cursor.header();
    payload[header.countries_at + 8..header.countries_at + 12].copy_from_slice(&1u32.to_le_bytes());
    payload[header.items_at..header.items_at + 4].copy_from_slice(&0u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report
        .findings
        .iter()
        .any(|finding| { finding.rule == crate::decode::D43DecodeRule::D43StockpileVariantRange }));
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43StockpileCountryCoverage)
    );
}

#[test]
fn mtrd_identity_order_precedes_half_predicate_order() {
    fn row(
        owner: u32,
        counterpart: u32,
        direction: DealDirection,
        exporter: u32,
        importer: u32,
        requested_raw: i64,
    ) -> DealRow {
        DealRow {
            owner_tag_db_id: owner,
            counterpart_tag_db_id: counterpart,
            resource_index: 0,
            visible_order: 0,
            direction: direction.wire(),
            route_kind: DealRoute::Sea.wire(),
            flags: DEAL_FLAG_EXACT,
            route_status_or_reserved: 0,
            identity_exporter_tag_db_id: exporter,
            identity_importer_tag_db_id: importer,
            identity_ordinal: 1,
            packed_identity: None,
            reserved_ordinal_word: None,
            requested_raw,
            delivered_raw: 0,
            available_export_raw: 0,
            factory_cost_raw: 0,
            efficiency_raw: 0,
            civilian_factories: 0,
            convoys_assigned: 0,
            convoys_required: 0,
        }
    }

    let rows = [
        row(1, 2, DealDirection::Export, 1, 2, FIXED_SCALE_RAW as i64),
        row(
            2,
            1,
            DealDirection::Import,
            1,
            2,
            2 * FIXED_SCALE_RAW as i64,
        ),
        row(3, 4, DealDirection::Export, 3, 4, FIXED_SCALE_RAW as i64),
    ];
    let mut payload = core_bytes(&[]).done();
    payload.extend_from_slice(
        &encode::encode_classic_deals(DealVersion::WideIdentity, &[1, 2, 3, 4], &[], &rows)
            .expect("review fixture encodes"),
    );
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let first = crate::decode::select_first_fatal(&report).expect("fatal deal finding");
    assert_eq!(
        first.rule,
        crate::decode::D43DecodeRule::D43DealHalfMismatch
    );
    assert_eq!(
        first.deal_identity,
        Some(crate::decode::D43DealIdentityOperands {
            exporter_tag_db_id: 1,
            importer_tag_db_id: 2,
            resource_index: 0,
            ordinal: 1,
        })
    );
}

#[test]
fn structural_core_row_truncation_names_the_core_rows_block() {
    let mut payload = core_bytes(&[[1, 0, 0]]).done();
    payload.truncate(CORE_HEADER_LEN);
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let structural = report
        .findings
        .iter()
        .find(|finding| finding.rule == crate::decode::D43DecodeRule::D43StructuralViolation)
        .expect("structural finding");
    assert_eq!(structural.block, crate::decode::D43DecodeBlock::D43CoreRows);
}

#[test]
fn truncated_core_rows_outrank_a_bad_core_scale() {
    let mut payload = core_bytes(&[[1, 0, 0]]).done();
    payload[44..48].copy_from_slice(&9u32.to_le_bytes());
    payload.truncate(CORE_HEADER_LEN);
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43StructuralViolation)
    );
}

#[test]
fn duplicate_trailer_finding_names_the_magic_block() {
    let mut b = core_bytes(&[]);
    mtra_v1(&mut b);
    mtra_v1(&mut b);
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    let structural = report
        .findings
        .iter()
        .find(|finding| finding.rule == crate::decode::D43DecodeRule::D43StructuralViolation)
        .expect("duplicate structural finding");
    assert_eq!(
        structural.block,
        crate::decode::D43DecodeBlock::D43TrailerMagic
    );
}

#[test]
fn deal_half_join_keys_distinct_resources_separately() {
    let mut b = core_bytes(&[]);
    mtrd_two_resources_same_parties(&mut b);
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(!report.findings.iter().any(|finding| matches!(
        finding.rule,
        crate::decode::D43DecodeRule::D43DealHalfCount
            | crate::decode::D43DecodeRule::D43DealHalfMismatch
    )));
}

#[test]
fn lane_only_families_are_refused_at_magic_or_fully_walked_by_policy() {
    for trailer in [Trailer::Residual, Trailer::LendLease] {
        let payload = a_fixture_with_every_block_filled(trailer);
        let shipping = crate::decode::decode_record(
            &payload,
            test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
        );
        assert!(
            matches!(shipping.structural_stop, Some(crate::decode::D43DecodeStop::D43RejectedFamily { trailer: found, .. }) if found == trailer)
        );
        match trailer {
            Trailer::Residual => assert!(shipping.document.residual_header.is_none()),
            Trailer::LendLease => assert!(shipping.document.lend_lease_header.is_none()),
            _ => unreachable!(),
        }
        let lane = crate::decode::decode_record(
            &payload,
            test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
        );
        assert_eq!(lane.structural_stop, None);
    }
}

#[test]
fn lane_only_family_predicates_are_fine_grained_and_nonfatal() {
    use crate::decode::{D43LaneRule, D43RuleFatality};

    let residual = crate::decode::decode_record(
        &a_fixture_with_every_block_filled(Trailer::Residual),
        test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
    );
    let lend_lease = crate::decode::decode_record(
        &a_fixture_with_every_block_filled(Trailer::LendLease),
        test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
    );
    assert!(residual.findings.iter().any(|finding| {
        finding.lane_rule == Some(D43LaneRule::D43LaneResidualGapIdZero)
            && finding.fatality == D43RuleFatality::NonFatal
    }));
    assert!(lend_lease.findings.iter().any(|finding| {
        finding.lane_rule == Some(D43LaneRule::D43LaneLendLeaseSourceRecordKind)
            && finding.fatality == D43RuleFatality::NonFatal
    }));
}

#[test]
fn residual_domain_and_row_width_predicates_are_leaf_owned_and_nonfatal() {
    use crate::decode::{D43LaneRule, D43RuleFatality};

    let mut payload = a_fixture_with_every_block_filled(Trailer::Residual);
    let row = CORE_HEADER_LEN + 32;
    payload[row + 16..row + 20].copy_from_slice(&999u32.to_le_bytes());
    payload[row + 24..row + 28].copy_from_slice(&513u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
    );
    for rule in [
        D43LaneRule::D43LaneResidualCurrentDomainUnknown,
        D43LaneRule::D43LaneResidualCurrentRowLenRange,
    ] {
        assert!(report.findings.iter().any(|finding| {
            finding.lane_rule == Some(rule) && finding.fatality == D43RuleFatality::NonFatal
        }));
    }
}

#[test]
fn lane_flow_country_count_ignores_unknown_kinds() {
    let mut payload = core_bytes(&[]);
    mtrf(
        &mut payload,
        FlowVersion::Variable,
        3,
        &[99, 99, 99, 99, 99, 99],
    );
    let report = crate::decode::decode_record(
        &payload.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
    );
    assert!(report.findings.iter().any(|finding| {
        finding.lane_rule == Some(crate::decode::D43LaneRule::D43LaneFlowDecodedCountryCount)
            && finding.operand_a == 0
            && finding.operand_b == 1
    }));
}

#[test]
fn lane_detail_speed_ceiling_is_a_shared_leaf_finding() {
    let mut b = core_bytes(&[]);
    let detail_at = b.at();
    mtd1_v1(&mut b);
    let mut payload = b.done();
    payload[detail_at + 32 + 24..detail_at + 32 + 28].copy_from_slice(&101i32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::LANE_ALL),
    );
    assert!(report.findings.iter().any(|finding| {
        finding.lane_rule == Some(crate::decode::D43LaneRule::D43LaneDetailSpeedRange)
            && finding.operand_a == 101
    }));
}

#[test]
fn cross_family_country_sets_are_compared_with_identity_only_exception() {
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Fixed7, 7, &[1, 2, 3, 4, 5, 6]);
    let mtrh_at = b.at();
    mtrh_v1(&mut b);
    let mut payload = b.done();
    payload[mtrh_at + 24..mtrh_at + 28].copy_from_slice(&22u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CrossCountrySetAgreement));

    let mut b = core_bytes(&[]);
    mtrh_v1(&mut b);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        2,
        &["oil", "steel"],
        &[],
        &[],
    );
    let identity_only = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(!identity_only
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CrossCountrySetAgreement));
}

#[test]
fn variable_flow_requires_identity_bearing_resource_rows() {
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 2, &[1, 2, 3, 4, 5, 6]);
    mtrr(&mut b, ResourceRowVersion::CountOnly, 2, &[], &[], &[]);
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CrossFlowV3NeedsIdentityResourceRows));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 2, &[1, 2, 3, 4, 5, 6]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        2,
        &["oil", "steel"],
        &[],
        &[],
    );
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(!report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43CrossFlowV3NeedsIdentityResourceRows));
}

#[test]
fn cross_family_absence_rules_wait_for_exact_consumption() {
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 2, &[1, 2, 3, 4, 5, 6]);
    b.magic(b"MTRR");
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report.structural_stop.is_some());
    assert!(!report.findings.iter().any(|finding| {
        finding.rule == crate::decode::D43DecodeRule::D43CrossFlowV3NeedsIdentityResourceRows
    }));
}

#[test]
fn flow_v3_double_fault_selects_resource_count_before_header_len() {
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 65, &[1, 2, 3, 4, 5, 6]);
    let mut payload = b.done();
    payload[CORE_HEADER_LEN + 6..CORE_HEADER_LEN + 8].copy_from_slice(&31u16.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(report.structural_stop.is_some());
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|f| f.rule),
        Some(crate::decode::D43DecodeRule::D43FlowHeaderResourceCount)
    );
}

#[test]
fn flow_v3_bad_resource_count_is_reported_once() {
    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 65, &[1, 2, 3, 4, 5, 6]);
    let report = crate::decode::decode_record(
        &b.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(
                |finding| finding.rule == crate::decode::D43DecodeRule::D43FlowHeaderResourceCount
            )
            .count(),
        1
    );
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.rule == crate::decode::D43DecodeRule::D43StructuralViolation));
}

#[test]
fn authority_units_distinguish_zero_from_not_reached() {
    let empty = crate::decode::decode_record(
        &core_bytes(&[]).done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(empty.units.market_trade_detail_rows, Some(0));

    let mut bad_scale = core_bytes(&[]).done();
    bad_scale[44..48].copy_from_slice(&9u32.to_le_bytes());
    let bad_scale = crate::decode::decode_record(
        &bad_scale,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(bad_scale.units.market_trade_contract_rows, None);
    assert_eq!(bad_scale.units.market_trade_detail_rows, None);

    let mut refused_detail = core_bytes(&[]).done();
    refused_detail.extend_from_slice(b"MTD1");
    let refused_detail = crate::decode::decode_record(
        &refused_detail,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(refused_detail.units.market_trade_contract_rows, Some(0));
    assert_eq!(refused_detail.units.market_trade_detail_rows, None);
}

#[test]
fn cross_family_refusal_retains_fully_decoded_family_units() {
    let mut payload = core_bytes(&[]);
    mtrf(
        &mut payload,
        FlowVersion::Fixed7,
        FIXED7_RESOURCE_COUNT,
        &[1, 2, 3, 4, 5, 6],
    );
    let factory_at = payload.at();
    mtrh_v1(&mut payload);
    let mut payload = payload.done();
    payload[CORE_HEADER_LEN + 32 + 4..CORE_HEADER_LEN + 32 + 8]
        .copy_from_slice(&99u32.to_le_bytes());
    payload[factory_at + 24..factory_at + 28].copy_from_slice(&22u32.to_le_bytes());
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(
        crate::decode::select_first_fatal(&report).map(|finding| finding.rule),
        Some(crate::decode::D43DecodeRule::D43FlowKind)
    );
    assert!(report.findings.iter().any(|finding| {
        finding.rule == crate::decode::D43DecodeRule::D43CrossCountrySetAgreement
    }));
    assert_eq!(report.units.market_trade_resource_flow_rows, Some(6));
    assert_eq!(report.units.market_trade_factory_header_rows, Some(1));

    let mut duplicate = core_bytes(&[]);
    mtra_v1(&mut duplicate);
    mtra_v1(&mut duplicate);
    let duplicate = crate::decode::decode_record(
        &duplicate.done(),
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(duplicate.structural_stop.is_some());
    assert_eq!(duplicate.units.market_trade_activation_rows, Some(1));
}

#[test]
fn large_malformed_core_bounds_findings_with_explicit_summaries() {
    const ROWS: usize = 4_096;
    let rows = vec![[0u64, 1u64 << 31, 0u64]; ROWS];
    let payload = core_bytes(&rows).done();
    let report = crate::decode::decode_record(
        &payload,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert_eq!(report.document.core_rows.len(), ROWS);
    assert!(report.findings.iter().any(|finding| {
        finding.rule == crate::decode::D43DecodeRule::D43FindingsSuppressed
            && finding.suppressed_rule == Some(crate::decode::D43DecodeRule::D43CoreContractIdZero)
            && finding.operand_a > 0
    }));
    for rule in [
        crate::decode::D43DecodeRule::D43CoreContractIdZero,
        crate::decode::D43DecodeRule::D43CoreDuplicateContractPair,
        crate::decode::D43DecodeRule::D43CoreUnknownFlags,
    ] {
        assert!(
            report
                .findings
                .iter()
                .filter(|finding| finding.rule == rule)
                .count()
                <= crate::decode::D43_MAX_FINDINGS_PER_RULE
        );
    }
    assert!(report.findings.len() < 1_000);
}

#[test]
fn legacy_flow_requires_at_least_one_nonzero_resource_value() {
    let mut b = core_bytes(&[]);
    mtrf(
        &mut b,
        FlowVersion::Legacy,
        FIXED7_RESOURCE_COUNT,
        &[FlowKind::Exported.wire()],
    );
    let valid = b.done();
    let valid_report = crate::decode::decode_record(
        &valid,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(!valid_report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43FlowLegacyValuesAllZero));

    let mut empty = valid;
    let values_at = CORE_HEADER_LEN + 32 + FLOW_ROW_HEADER_LEN;
    empty[values_at..values_at + FIXED7_RESOURCE_COUNT as usize * 8].fill(0);
    let empty_report = crate::decode::decode_record(
        &empty,
        test_decode_context(crate::decode::D43AcceptedFamilies::SHIPPING),
    );
    assert!(empty_report
        .findings
        .iter()
        .any(|f| f.rule == crate::decode::D43DecodeRule::D43FlowLegacyValuesAllZero));
}

#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn new() -> Self {
        Bytes(Vec::new())
    }
    fn at(&self) -> usize {
        self.0.len()
    }
    fn magic(&mut self, m: &[u8; 4]) -> &mut Self {
        self.0.extend_from_slice(m);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        for _ in 0..n {
            self.0.push(0);
        }
        self
    }
    fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn done(&self) -> Vec<u8> {
        self.0.clone()
    }
}

fn core_bytes(rows: &[[u64; 3]]) -> Bytes {
    let mut b = Bytes::new();
    b.u64(7);
    b.u32(rows.len() as u32);
    b.u32(0);
    b.u32(0);
    b.u32(rows.len() as u32);
    b.u32(rows.len() as u32);
    b.u32(0);
    b.u32(0);
    b.u32(0);
    b.u32(rows.len() as u32);
    b.u32(FIXED_SCALE_RAW);
    for row in rows {
        b.u32(row[0] as u32);
        b.u32(CORE_CONTRACT_TYPE);
        b.u32(row[1] as u32);
        b.u32(11);
        b.u32(12);
        b.u32(0);
        b.i64(row[2] as i64);
        b.u32(0);
        b.u32(0);
        b.u32(0);
        b.u32(0);
    }
    b
}

trait WalkBlockToEndForTest<N> {
    fn walk_to_end_for_test(self) -> BlockEnd<N>;
}

impl<N> WalkBlockToEndForTest<N> for BlockCursor<N> {
    fn walk_to_end_for_test(mut self) -> BlockEnd<N> {
        while let RowStep::Row { .. } = self.next() {}
        self.into_end()
            .expect("the test helper walked every declared row")
    }
}

fn walk_body_to_end_for_test(body: TrailerBody) -> TrailerResume {
    match body {
        TrailerBody::Detail(c) => c.rows().walk_to_end_for_test().next,
        TrailerBody::ResourceFlow(c) => c.rows().walk_to_end_for_test().next,
        TrailerBody::FactoryHeader(c) => c.rows().walk_to_end_for_test().next,
        TrailerBody::Residual(c) => c.rows().walk_to_end_for_test().next,
        TrailerBody::LendLease(c) => c.rows().walk_to_end_for_test().next,
        TrailerBody::ResourceRow(c) => {
            c.identities()
                .walk_to_end_for_test()
                .next
                .rows()
                .walk_to_end_for_test()
                .next
        }
        TrailerBody::ClassicDeal(c) => {
            c.witnesses()
                .walk_to_end_for_test()
                .next
                .incomplete_owners()
                .walk_to_end_for_test()
                .next
                .rows()
                .walk_to_end_for_test()
                .next
        }
        TrailerBody::Stockpile(c) => {
            c.countries()
                .walk_to_end_for_test()
                .next
                .items()
                .walk_to_end_for_test()
                .next
        }
        TrailerBody::Activation(c) => c.end().next,
    }
}

fn visit_trailers<F>(payload: &[u8], mut visit: F) -> Result<usize, Violation>
where
    F: FnMut(&[u8], usize, TrailerBody) -> Result<TrailerResume, Violation>,
{
    let header = read_core_header(payload)?;
    let core = core_end(payload, header.row_count())?;
    let mut cursor = TrailerCursor::new(core);
    loop {
        match cursor.next(payload)? {
            TrailerStep::End(at) => return Ok(at),
            TrailerStep::Magic { at, .. } => {
                let body = cursor.read_body(payload, TEST_LIMITS)?;
                cursor = visit(payload, at, body)?.resume();
            }
        }
    }
}

fn walk_all(payload: &[u8]) -> Result<Vec<(Trailer, usize, usize)>, Violation> {
    let mut seen = Vec::new();
    let end = visit_trailers(payload, |_, at, body| {
        let trailer = body.trailer();
        let resume = walk_body_to_end_for_test(body);
        seen.push((trailer, at, resume.end()));
        Ok(resume)
    })?;
    exact_consumption(end, payload.len())?;
    Ok(seen)
}

#[test]
fn every_declared_trailer_has_a_handwritten_fixture() {
    let mut covered = Vec::new();
    for (payload, _) in every_fixture() {
        for (trailer, _, _) in walk_all(&payload).expect("fixture walks") {
            if !covered.contains(&trailer) {
                covered.push(trailer);
            }
        }
    }
    for trailer in Trailer::ALL {
        assert!(
            covered.contains(&trailer),
            "no handwritten fixture carries {}",
            trailer.label()
        );
    }
}

#[test]
fn every_declared_version_has_a_handwritten_fixture() {
    let mut flow = Vec::new();
    let mut resource = Vec::new();
    let mut deal = Vec::new();
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |_, _, body| {
            match &body {
                TrailerBody::ResourceFlow(c) => flow.push(c.header().version),
                TrailerBody::ResourceRow(c) => resource.push(c.header().version),
                TrailerBody::ClassicDeal(c) => deal.push(c.header().version),
                _ => {}
            }
            Ok(walk_body_to_end_for_test(body))
        })
        .expect("walk");
    }
    for v in FlowVersion::ALL {
        assert!(flow.contains(&v), "no fixture carries MTRF {:?}", v);
    }
    for v in ResourceRowVersion::ALL {
        assert!(resource.contains(&v), "no fixture carries MTRR {:?}", v);
    }
    for v in DealVersion::ALL {
        assert!(deal.contains(&v), "no fixture carries MTRD {:?}", v);
    }
}

#[test]
fn core_flag_fixtures_cover_exactly_the_known_mask() {
    let mut union = 0u32;
    for (payload, _) in every_fixture() {
        let header = read_core_header(&payload).expect("header");
        let mut rows = CoreRowCursor::new(header.row_count());
        while let CoreRowStep::Row { row, .. } = rows.next(&payload) {
            union |= row.flags;
        }
    }
    assert_eq!(
        union, KNOWN_ROW_FLAGS,
        "core-row fixtures cover {union:#x}, the table declares {KNOWN_ROW_FLAGS:#x}"
    );
}

#[test]
fn detail_flag_fixtures_cover_exactly_the_known_mask() {
    let mut union = 0u32;
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| {
            let TrailerBody::Detail(c) = body else {
                return Ok(walk_body_to_end_for_test(body));
            };
            let mut rows = c.rows();
            while let RowStep::Row { at, .. } = rows.next() {
                union |= detail_row_at(payload, at).flags;
            }
            Ok(rows.into_end()?.next)
        })
        .expect("walk");
    }
    assert_eq!(union, KNOWN_DETAIL_FLAGS);
}

#[test]
fn resource_row_fixtures_cover_every_flag_and_needed_type() {
    let mut union = 0u32;
    let mut kinds = Vec::new();
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| {
            let TrailerBody::ResourceRow(c) = body else {
                return Ok(walk_body_to_end_for_test(body));
            };
            let mut rows = c.identities().walk_to_end_for_test().next.rows();
            while let RowStep::Row { at, .. } = rows.next() {
                let row = resource_row_at(payload, at);
                union |= row.flags;
                if let Some(kind) = NeededType::from_wire(row.needed_type) {
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
            }
            Ok(rows.into_end()?.next)
        })
        .expect("walk");
    }
    assert_eq!(union, KNOWN_RESOURCE_FLAGS);
    for kind in NeededType::ALL {
        assert!(kinds.contains(&kind), "no fixture states {:?}", kind);
    }
}

#[test]
fn deal_and_flow_vocabularies_are_covered() {
    let mut flow_kinds = Vec::new();
    let mut directions = Vec::new();
    let mut routes = Vec::new();
    let mut status_union = 0u32;
    for (payload, _) in every_fixture() {
        visit_trailers(&payload, |payload, _, body| match body {
            TrailerBody::ResourceFlow(c) => {
                let mut rows = c.rows();
                while let RowStep::Row { at, .. } = rows.next() {
                    if let Some(kind) = FlowKind::from_wire(flow_row_at(payload, at).flow_kind) {
                        if !flow_kinds.contains(&kind) {
                            flow_kinds.push(kind);
                        }
                    }
                }
                Ok(rows.into_end()?.next)
            }
            TrailerBody::ClassicDeal(c) => {
                let version = c.header().version;
                let mut rows = c
                    .witnesses()
                    .walk_to_end_for_test()
                    .next
                    .incomplete_owners()
                    .walk_to_end_for_test()
                    .next
                    .rows();
                while let RowStep::Row { at, .. } = rows.next() {
                    let row = deal_row_at(payload, at, version);
                    if let Some(d) = DealDirection::from_wire(row.direction) {
                        if !directions.contains(&d) {
                            directions.push(d);
                        }
                    }
                    if let Some(r) = DealRoute::from_wire(row.route_kind) {
                        if !routes.contains(&r) {
                            routes.push(r);
                        }
                    }
                    if version.carries_route_status() {
                        status_union |= row.route_status_or_reserved;
                    }
                }
                Ok(rows.into_end()?.next)
            }
            _ => Ok(walk_body_to_end_for_test(body)),
        })
        .expect("walk");
    }
    for kind in FlowKind::ALL {
        assert!(flow_kinds.contains(&kind), "no fixture states {:?}", kind);
    }
    for d in DealDirection::ALL {
        assert!(directions.contains(&d), "no fixture states {:?}", d);
    }
    for r in DealRoute::ALL {
        assert!(routes.contains(&r), "no fixture states {:?}", r);
    }
    assert_eq!(status_union, KNOWN_DEAL_STATUS);
}

fn mtd1_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTD1");
    b.u16(1);
    b.u16(32);
    b.u16(64);
    b.u16(0);
    b.u32(2);
    b.u32(1);
    b.u32(1);
    b.u32(1);
    b.u32(1);
    for (contract_id, flags) in [(1u32, 0b0000011111u32), (2u32, 0b1111100000u32)] {
        b.u32(contract_id);
        b.u32(flags);
        b.i64(100_000);
        b.i64(200_000);
        b.i32(3);
        b.i32(4);
        b.i32(5);
        b.u32(12);
        b.i32(6);
        b.i32(7);
        b.i32(8);
        b.i32(9);
        b.i32(10);
        b.u32(0);
    }
    assert_eq!(b.at() - start, 32 + 2 * 64);
}

fn mtrf(b: &mut Bytes, version: FlowVersion, resource_count: u32, kinds: &[u32]) {
    let start = b.at();
    let (row_len, mask_or_count, kind_mask) = match version {
        FlowVersion::Legacy => (72usize, FLOW_RESOURCE_MASK, 0u32),
        FlowVersion::Fixed7 => (72, FLOW_RESOURCE_MASK, FLOW_KIND_MASK),
        FlowVersion::Variable => (
            16 + 8 * resource_count as usize,
            resource_count,
            FLOW_KIND_MASK,
        ),
    };
    let values = match version {
        FlowVersion::Legacy | FlowVersion::Fixed7 => 7,
        FlowVersion::Variable => resource_count as usize,
    };
    b.magic(b"MTRF");
    b.u16(version.wire());
    b.u16(32);
    b.u16(row_len as u16);
    b.u16(0);
    b.u32(kinds.len() as u32);
    b.u32(match version {
        FlowVersion::Legacy => kinds.len() as u32,
        FlowVersion::Fixed7 | FlowVersion::Variable => (kinds.len() / 6) as u32,
    });
    b.u32(FIXED_SCALE_RAW);
    b.u32(mask_or_count);
    b.u32(kind_mask);
    for kind in kinds {
        b.u32(21);
        b.u32(*kind);
        b.u32(mask_or_count);
        b.u32(0);
        for slot in 0..values {
            b.i64(1_000 + slot as i64);
        }
    }
    assert_eq!(b.at() - start, 32 + kinds.len() * row_len);
}

fn mtrh_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRH");
    b.u16(1);
    b.u16(24);
    b.u16(16);
    b.u16(0);
    b.u32(1);
    b.u32(1);
    b.u32(0);
    b.u32(21);
    b.i32(10);
    b.i32(4);
    b.i32(6);
    assert_eq!(b.at() - start, 24 + 16);
}

fn mtrr(
    b: &mut Bytes,
    version: ResourceRowVersion,
    resource_count: u32,
    tokens: &[&str],
    countries: &[u32],
    rows: &[(u32, u32, u32)],
) {
    let start = b.at();
    let stride = TEST_LIMITS.identity_stride();
    b.magic(b"MTRR");
    b.u16(version.wire());
    b.u16(32);
    b.u16(64);
    b.u16(0);
    b.u32((countries.len() * rows.len()) as u32);
    b.u32(countries.len() as u32);
    b.u32(resource_count);
    b.u32(1);
    b.u32(if version.carries_identity() {
        stride as u32
    } else {
        0
    });
    for token in tokens {
        b.u16(token.len() as u16);
        b.raw(token.as_bytes());
        b.zeros(stride - 2 - token.len());
    }
    for country in countries {
        for (resource_index, needed_type, flags) in rows {
            b.u32(*country);
            b.u32(*resource_index);
            b.u32(*resource_index);
            b.u32(*needed_type);
            b.i32(5);
            b.i32(5);
            b.i32(0);
            b.i32(0);
            b.i32(0);
            b.i32(0);
            b.i32(5);
            b.i32(0);
            b.i32(0);
            b.u32(*flags);
            b.u32(resource_index + 1);
            b.u32(0);
        }
    }
    let identity = if version.carries_identity() {
        resource_count as usize * stride
    } else {
        0
    };
    assert_eq!(
        b.at() - start,
        32 + identity + countries.len() * rows.len() * 64
    );
}

fn mtrd(b: &mut Bytes, version: DealVersion, witnesses: &[u32], incomplete: &[u32]) {
    mtrd_with_routes(
        b,
        version,
        witnesses,
        incomplete,
        [DealRoute::Sea.wire(), DealRoute::Land.wire()],
    )
}

fn mtrd_with_routes(
    b: &mut Bytes,
    version: DealVersion,
    witnesses: &[u32],
    incomplete: &[u32],
    routes: [u32; 2],
) {
    let start = b.at();
    b.magic(b"MTRD");
    b.u16(version.wire());
    b.u16(32);
    b.u16(96);
    b.u16(0);
    b.u32(2);
    b.u32(witnesses.len() as u32);
    b.u32(FIXED_SCALE_RAW);
    b.u32(witnesses.len() as u32);
    b.u32(incomplete.len() as u32);
    for w in witnesses {
        b.u32(*w);
    }
    for owner in incomplete {
        b.u32(*owner);
    }
    let exporter = witnesses[0];
    let importer = witnesses[1];
    for (half, (owner, counterpart, direction, route, status)) in [
        (
            exporter,
            importer,
            DealDirection::Export.wire(),
            routes[0],
            DealStatus::NoTradeInfluence.bit(),
        ),
        (
            importer,
            exporter,
            DealDirection::Import.wire(),
            routes[1],
            DealStatus::NoResources.bit(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = half;
        b.u32(owner);
        b.u32(counterpart);
        b.u32(1);
        b.u32(0);
        b.u32(direction);
        b.u32(route);
        b.u32(DEAL_FLAG_EXACT);
        b.u32(if version.carries_route_status() {
            status
        } else {
            0
        });
        match version.identity_shape() {
            DealIdentityShape::Wide => {
                b.u32(exporter);
                b.u32(importer);
            }
            DealIdentityShape::Packed => {
                let packed =
                    ((exporter as u64) << 32) | ((importer as u64) << 16) | (1u64 << 8) | 1u64;
                b.u64(packed);
            }
        }
        b.i64(FIXED_SCALE_RAW as i64);
        b.i64(0);
        b.i64(0);
        b.i64(0);
        b.i64(0);
        b.i32(0);
        b.i32(0);
        b.i32(0);
        match version.identity_shape() {
            DealIdentityShape::Wide => b.u32(1),
            DealIdentityShape::Packed => b.u32(0),
        };
    }
    assert_eq!(
        b.at() - start,
        32 + witnesses.len() * 4 + incomplete.len() * 4 + 2 * 96
    );
}

fn mtrd_two_resources_same_parties(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRD");
    b.u16(DealVersion::WideIdentity.wire());
    b.u16(32);
    b.u16(96);
    b.u16(0);
    b.u32(4);
    b.u32(2);
    b.u32(FIXED_SCALE_RAW);
    b.u32(2);
    b.u32(0);
    b.u32(21).u32(22);
    for (visible_order, resource_index, owner, counterpart, direction) in [
        (0, 0, 21, 22, DealDirection::Export.wire()),
        (0, 0, 22, 21, DealDirection::Import.wire()),
        (1, 1, 21, 22, DealDirection::Export.wire()),
        (1, 1, 22, 21, DealDirection::Import.wire()),
    ] {
        b.u32(owner);
        b.u32(counterpart);
        b.u32(resource_index);
        b.u32(visible_order);
        b.u32(direction);
        b.u32(DealRoute::Sea.wire());
        b.u32(DEAL_FLAG_EXACT);
        b.u32(0);
        b.u32(21);
        b.u32(22);
        b.i64(FIXED_SCALE_RAW as i64);
        b.i64(0);
        b.i64(0);
        b.i64(0);
        b.i64(0);
        b.i32(0);
        b.i32(0);
        b.i32(0);
        b.u32(1);
    }
    assert_eq!(b.at() - start, 32 + 8 + 4 * 96);
}

fn mtra_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRA");
    b.u16(1);
    b.u16(32);
    b.u32(0b11);
    b.u32(21);
    b.u64(9);
    b.u32(1);
    b.u32(0x015e_c520);
    assert_eq!(b.at() - start, 32);
}

fn mts1_v1(b: &mut Bytes) {
    let start = b.at();
    let segment = 40 + 1 * 20 + 2 * 24;
    b.magic(b"MTS1");
    b.u16(1);
    b.u16(40);
    b.u16(20);
    b.u16(24);
    b.u32(segment as u32);
    b.u32(1);
    b.u32(2);
    b.u32(1);
    b.u32(FIXED_SCALE_RAW);
    b.u32(STOCKPILE_GLOBAL_FLAGS);
    b.u32(0);
    b.u32(21);
    b.u32(0);
    b.u32(2);
    b.u32(STOCKPILE_COUNTRY_COMPLETE);
    b.u32(0);
    for ordinal in 0..2u32 {
        b.u32(100 + ordinal);
        b.u32(STOCKPILE_EQUIPMENT_TYPE);
        b.u32(ordinal);
        b.u32(0);
        b.i64(5 * FIXED_SCALE_RAW as i64);
    }
    assert_eq!(b.at() - start, segment);
}

fn mtrb_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTRB");
    b.u16(1);
    b.u16(32);
    b.u16(48);
    b.u16(0);
    b.u32(1);
    b.u32(1);
    b.u32(0);
    b.u32(0);
    b.u32(0);
    for word in 0..12u32 {
        b.u32(word);
    }
    assert_eq!(b.at() - start, 32 + 48);
}

fn mtll_v1(b: &mut Bytes) {
    let start = b.at();
    b.magic(b"MTLL");
    b.u16(1);
    b.u16(32);
    b.u16(48);
    b.u16(0);
    b.u32(1);
    b.u32(1);
    b.u32(0);
    b.u32(1);
    b.u32(0);
    b.u32(1);
    b.u32(11);
    b.u32(12);
    b.u32(1);
    b.u32(100);
    b.u32(1);
    b.i64(FIXED_SCALE_RAW as i64);
    b.i64(0);
    b.u32(0);
    b.u32(0);
    assert_eq!(b.at() - start, 32 + 48);
}

fn every_fixture() -> Vec<(Vec<u8>, &'static str)> {
    let mut out = Vec::new();

    out.push((core_bytes(&[[1, 0b11111, 5]]).done(), "core only"));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    out.push((b.done(), "MTRF v3 alone, no MTD1 predecessor"));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Legacy, 7, &[FlowKind::Exported.wire()]);
    out.push((b.done(), "MTRF v1 legacy fixed-seven"));

    let mut b = core_bytes(&[]);
    mtrf(&mut b, FlowVersion::Fixed7, 7, &[1, 2, 3, 4, 5, 6]);
    out.push((b.done(), "MTRF v2 fixed-seven six-kind matrix"));

    let mut b = core_bytes(&[]);
    mtd1_v1(&mut b);
    out.push((b.done(), "MTD1 v1"));

    let mut b = core_bytes(&[]);
    mtrh_v1(&mut b);
    out.push((b.done(), "MTRH v1"));

    let mut b = core_bytes(&[]);
    let seven: Vec<(u32, u32, u32)> = (0..7)
        .map(|i| {
            if i == 0 {
                (i, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL)
            } else {
                (i, NeededType::Surplus.wire(), 0)
            }
        })
        .collect();
    mtrr(&mut b, ResourceRowVersion::Legacy, 7, &[], &[21], &seven);
    out.push((b.done(), "MTRR v1 fixed-seven, no identity"));

    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        3,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
            (2, NeededType::Deficit.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v2 count-only, no identity"));

    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[],
        &[],
    );
    out.push((b.done(), "MTRR v3 identity-only, zero rows"));

    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[21, 22],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
            (2, NeededType::Deficit.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v3 identity plus dense rows"));

    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::NeededUnavailable,
        2,
        &["oil", "steel"],
        &[21],
        &[
            (
                0,
                NeededType::Unavailable.wire(),
                ResourceFlag::NeededUnavailable.bit(),
            ),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    out.push((b.done(), "MTRR v4 needed-unavailable"));

    for version in DealVersion::ALL {
        let mut b = core_bytes(&[]);
        let incomplete: &[u32] = if version.carries_incomplete_owners() {
            &[22]
        } else {
            &[]
        };
        mtrd(&mut b, version, &[21, 22], incomplete);
        out.push((
            b.done(),
            match version {
                DealVersion::Legacy => "MTRD v1 packed identity",
                DealVersion::Statuses => "MTRD v2 packed identity with route status",
                DealVersion::PerOwner => "MTRD v3 witnesses plus incomplete owners",
                DealVersion::WideIdentity => "MTRD v4 wide identity",
            },
        ));
    }

    let mut b = core_bytes(&[]);
    mtrd_with_routes(
        &mut b,
        DealVersion::WideIdentity,
        &[21, 22],
        &[],
        [DealRoute::Locked.wire(), DealRoute::Locked.wire()],
    );
    out.push((b.done(), "MTRD v4 with a locked route"));

    let mut b = core_bytes(&[]);
    mtra_v1(&mut b);
    out.push((b.done(), "MTRA v1"));

    let mut b = core_bytes(&[]);
    mts1_v1(&mut b);
    out.push((b.done(), "MTS1 v1 with country and item rows"));

    let mut b = core_bytes(&[]);
    mtrb_v1(&mut b);
    out.push((b.done(), "MTRB v1 (lane gate only)"));

    let mut b = core_bytes(&[]);
    mtll_v1(&mut b);
    out.push((b.done(), "MTLL v1 (lane gate only)"));

    let mut b = core_bytes(&[[1, 0b11111, 5]]);
    mtd1_v1(&mut b);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    mtrh_v1(&mut b);
    mtra_v1(&mut b);
    mts1_v1(&mut b);
    out.push((b.done(), "producer order"));

    let mut b = core_bytes(&[[1, 0b11111, 5]]);
    mts1_v1(&mut b);
    mtra_v1(&mut b);
    mtrh_v1(&mut b);
    mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]);
    mtd1_v1(&mut b);
    out.push((b.done(), "reverse producer order"));

    out
}

#[test]
fn every_fixture_walks_to_exact_consumption() {
    for (payload, name) in every_fixture() {
        let seen = walk_all(&payload).unwrap_or_else(|v| panic!("{name}: {v:?}"));
        if let Some((_, _, end)) = seen.last() {
            assert_eq!(*end, payload.len(), "{name} did not consume its payload");
        }
    }
}

#[test]
fn trailer_order_does_not_change_what_is_delimited() {
    let fixtures = every_fixture();
    let forward = fixtures
        .iter()
        .find(|(_, n)| *n == "producer order")
        .expect("fixture");
    let reverse = fixtures
        .iter()
        .find(|(_, n)| *n == "reverse producer order")
        .expect("fixture");
    let mut a: Vec<Trailer> = walk_all(&forward.0)
        .expect("forward")
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    let mut b: Vec<Trailer> = walk_all(&reverse.0)
        .expect("reverse")
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(forward.0.len(), reverse.0.len());
}

#[test]
fn a_short_core_header_is_refused() {
    let err = read_core_header(&[0u8; 47]).expect_err("short header");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::CoreHeader,
            needed: CORE_HEADER_LEN,
            available: 47
        }
    ));
}

#[test]
fn a_core_shorter_than_its_own_row_count_is_refused() {
    let mut payload = core_bytes(&[[1, 0, 0]]).done();
    payload.truncate(CORE_HEADER_LEN + 10);
    let header = read_core_header(&payload).expect("header");
    let err = core_end(&payload, header.row_count()).expect_err("short rows");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::CoreRows,
            ..
        }
    ));
}

#[test]
fn an_unknown_magic_is_refused_before_any_header_is_read() {
    let mut b = core_bytes(&[]);
    b.magic(b"ZZZZ").u16(1).u16(32).u16(64).u16(0).u32(0);
    let err = walk_all(&b.done()).expect_err("unknown magic");
    assert!(matches!(
        err.kind,
        ViolationKind::UnknownTrailerMagic { .. }
    ));
}

#[test]
fn a_truncated_magic_is_refused_rather_than_treated_as_the_end() {
    let mut b = core_bytes(&[]);
    b.raw(b"MTD");
    let err = walk_all(&b.done()).expect_err("truncated magic");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::TrailerMagic,
            ..
        }
    ));
}

#[test]
fn every_family_refuses_a_duplicate() {
    for trailer in Trailer::ALL {
        let mut b = core_bytes(&[]);
        let write = |b: &mut Bytes| match trailer {
            Trailer::Detail => mtd1_v1(b),
            Trailer::ResourceFlow => mtrf(b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]),
            Trailer::FactoryHeader => mtrh_v1(b),
            Trailer::ResourceRow => mtrr(
                b,
                ResourceRowVersion::Identity,
                2,
                &["oil", "steel"],
                &[21],
                &[
                    (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                    (1, NeededType::Surplus.wire(), 0),
                ],
            ),
            Trailer::ClassicDeal => mtrd(b, DealVersion::WideIdentity, &[21, 22], &[22]),
            Trailer::Activation => mtra_v1(b),
            Trailer::Stockpile => mts1_v1(b),
            Trailer::Residual => mtrb_v1(b),
            Trailer::LendLease => mtll_v1(b),
        };
        write(&mut b);
        write(&mut b);
        let err = walk_all(&b.done())
            .unwrap_err_or_panic(&format!("{} duplicate accepted", trailer.label()));
        assert!(
            matches!(err.kind, ViolationKind::DuplicateTrailer { trailer: t, .. } if t == trailer),
            "{} produced {:?}",
            trailer.label(),
            err.kind
        );
    }
}

trait UnwrapErrOrPanic<T> {
    fn unwrap_err_or_panic(self, msg: &str) -> Violation;
}

impl<T> UnwrapErrOrPanic<T> for Result<T, Violation> {
    fn unwrap_err_or_panic(self, msg: &str) -> Violation {
        match self {
            Ok(_) => panic!("{msg}"),
            Err(v) => v,
        }
    }
}

#[test]
fn an_unknown_version_is_refused_rather_than_reinterpreted() {
    for (magic, header_len, row_len) in [
        (b"MTD1", 32u16, 64u16),
        (b"MTRF", 32, 72),
        (b"MTRH", 24, 16),
        (b"MTRR", 32, 64),
        (b"MTRD", 32, 96),
        (b"MTRA", 32, 0),
        (b"MTS1", 40, 20),
    ] {
        let mut b = core_bytes(&[]);
        b.magic(magic).u16(99).u16(header_len).u16(row_len).u16(0);
        b.zeros(header_len as usize - 12);
        let err = walk_all(&b.done()).unwrap_err_or_panic("a version 99 trailer was accepted");
        assert!(
            matches!(err.kind, ViolationKind::UnknownVersion { version: 99, .. }),
            "{:?}",
            err.kind
        );
    }
}

#[test]
fn a_declared_header_len_that_is_not_the_versions_is_refused() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTD1").u16(1).u16(64).u16(64).u16(0).u32(0);
    b.zeros(16);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a wrong header_len was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::HeaderLen {
            declared: 64,
            expected: 32,
            ..
        }
    ));
}

#[test]
fn a_declared_row_len_that_is_not_the_versions_is_refused() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTD1").u16(1).u16(32).u16(48).u16(0).u32(0);
    b.zeros(16);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a wrong row_len was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::RowLen {
            declared: 48,
            expected: 64,
            ..
        }
    ));
}

#[test]
fn an_mtrf_v3_resource_count_outside_the_ceiling_is_refused() {
    for count in [0u32, TEST_LIMITS.max_resource_count() + 1] {
        let mut b = core_bytes(&[]);
        b.magic(b"MTRF")
            .u16(3)
            .u16(32)
            .u16(flow_row_len(count) as u16)
            .u16(0)
            .u32(0)
            .u32(0)
            .u32(FIXED_SCALE_RAW)
            .u32(count)
            .u32(FLOW_KIND_MASK);
        let err = walk_all(&b.done()).unwrap_err_or_panic("an out-of-range count was accepted");
        assert!(
            matches!(err.kind, ViolationKind::ResourceCount { declared, .. } if declared == count),
            "{:?}",
            err.kind
        );
    }
}

#[test]
fn an_mtrr_without_an_identity_block_may_not_declare_one() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTRR")
        .u16(ResourceRowVersion::CountOnly.wire())
        .u16(32)
        .u16(64)
        .u16(0)
        .u32(0)
        .u32(0)
        .u32(3)
        .u32(1)
        .u32(TEST_LIMITS.identity_stride() as u32);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a v2 identity stride was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::AbsentBlockDeclared { .. }
    ));
}

#[test]
fn an_mtrd_without_an_incomplete_block_may_not_declare_one() {
    let mut b = core_bytes(&[]);
    b.magic(b"MTRD")
        .u16(DealVersion::Statuses.wire())
        .u16(32)
        .u16(96)
        .u16(0)
        .u32(0)
        .u32(0)
        .u32(FIXED_SCALE_RAW)
        .u32(0)
        .u32(1);
    let err = walk_all(&b.done()).unwrap_err_or_panic("a v2 incomplete count was accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::AbsentBlockDeclared { declared: 1, .. }
    ));
}

#[test]
fn every_family_refuses_a_payload_one_byte_short() {
    for (payload, name) in every_fixture() {
        if payload.len() <= CORE_HEADER_LEN {
            continue;
        }
        let mut short = payload.clone();
        short.pop();
        let err = walk_all(&short)
            .unwrap_err_or_panic(&format!("{name} accepted a payload one byte short"));
        assert!(
            matches!(
                err.kind,
                ViolationKind::Need { .. } | ViolationKind::TrailingBytes { .. }
            ),
            "{name}: {:?}",
            err.kind
        );
    }
}

#[test]
fn trailing_bytes_past_the_last_trailer_are_refused() {
    let mut b = core_bytes(&[]);
    mtra_v1(&mut b);
    let mut payload = b.done();
    payload.extend_from_slice(&[0, 0, 0]);
    let err = walk_all(&payload).unwrap_err_or_panic("trailing bytes accepted");
    assert!(matches!(
        err.kind,
        ViolationKind::Need {
            section: Section::TrailerMagic,
            ..
        }
    ));
}

#[test]
fn the_encoder_reproduces_the_handwritten_core_bytes() {
    let rows = [CoreRow {
        contract_id: 1,
        contract_type: CORE_CONTRACT_TYPE,
        flags: 0b11111,
        seller_tag_db_id: 11,
        buyer_tag_db_id: 12,
        equipment_variant_id: 0,
        amount_raw: 5,
        equipment_subbuf_status: 0,
        delivery_subbuf_status: 0,
        buyer_ptr_status: 0,
        reserved: 0,
    }];
    let encoded = encode::encode_core(
        7,
        encode::CoreCounts {
            changed_rows: 0,
            status_flags: 0,
            observed_contract_rows: 1,
            emitted_contract_rows: 1,
            buyer_ptr_valid_rows: 0,
            variant_valid_rows: 0,
            amount_valid_rows: 0,
            partial_field_rows: 1,
        },
        &rows,
    )
    .expect("encodes");
    assert_eq!(encoded, core_bytes(&[[1, 0b11111, 5]]).done());
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtra_bytes() {
    let mut b = Bytes::new();
    mtra_v1(&mut b);
    assert_eq!(
        encode::encode_activation(0b11, 21, 9, 1, 0x015e_c520),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mts1_bytes() {
    let mut b = Bytes::new();
    mts1_v1(&mut b);
    let countries = [StockpileCountryRow {
        tag_db_id: 21,
        item_first: 0,
        item_count: 2,
        flags: STOCKPILE_COUNTRY_COMPLETE,
        reserved: 0,
    }];
    let items: Vec<StockpileItemRow> = (0..2u32)
        .map(|ordinal| StockpileItemRow {
            equipment_variant_id: 100 + ordinal,
            variant_type: STOCKPILE_EQUIPMENT_TYPE,
            ordinal,
            reserved: 0,
            amount_raw: 5 * FIXED_SCALE_RAW as i64,
        })
        .collect();
    assert_eq!(
        encode::encode_stockpile(1, &countries, &items).expect("encodes"),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrh_bytes() {
    let mut b = Bytes::new();
    mtrh_v1(&mut b);
    let rows = [FactoryRow {
        tag_db_id: 21,
        total: 10,
        used: 4,
        available: 6,
    }];
    assert_eq!(
        encode::encode_factory_header(&rows).expect("encodes"),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrf_bytes_at_every_version() {
    for (version, resource_count, kinds) in [
        (FlowVersion::Legacy, 7u32, vec![FlowKind::Exported.wire()]),
        (FlowVersion::Fixed7, 7, vec![1, 2, 3, 4, 5, 6]),
        (FlowVersion::Variable, 3, vec![1, 2, 3, 4, 5, 6]),
    ] {
        let mut b = Bytes::new();
        mtrf(&mut b, version, resource_count, &kinds);
        let values: Vec<i64> = (0..match version {
            FlowVersion::Legacy | FlowVersion::Fixed7 => 7,
            FlowVersion::Variable => resource_count as usize,
        })
            .map(|slot| 1_000 + slot as i64)
            .collect();
        let rows: Vec<encode::FlowRowInput<'_>> = kinds
            .iter()
            .map(|kind| encode::FlowRowInput {
                tag_db_id: 21,
                flow_kind: *kind,
                values_raw: &values,
            })
            .collect();
        let country_word = match version {
            FlowVersion::Legacy => kinds.len() as u32,
            FlowVersion::Fixed7 | FlowVersion::Variable => (kinds.len() / 6) as u32,
        };
        assert_eq!(
            encode::encode_resource_flow(version, resource_count, country_word, &rows, TEST_LIMITS)
                .expect("encodes"),
            b.done(),
            "MTRF {version:?}"
        );
    }
}

#[test]
fn the_encoder_refuses_the_structurally_impossible_mtrd_shapes() {
    let err = encode::encode_classic_deals(DealVersion::Statuses, &[21, 22], &[22], &[])
        .expect_err("v2 cannot carry incomplete owners");
    assert!(matches!(
        err,
        encode::EncodeError::IncompleteOwnersUnsupported {
            version: 2,
            owners: 1
        }
    ));

    let row = DealRow {
        owner_tag_db_id: 21,
        counterpart_tag_db_id: 22,
        resource_index: 1,
        visible_order: 0,
        direction: DealDirection::Export.wire(),
        route_kind: DealRoute::Sea.wire(),
        flags: DEAL_FLAG_EXACT,
        route_status_or_reserved: 0,
        identity_exporter_tag_db_id: 70_000,
        identity_importer_tag_db_id: 22,
        identity_ordinal: 1,
        packed_identity: None,
        reserved_ordinal_word: None,
        requested_raw: 0,
        delivered_raw: 0,
        available_export_raw: 0,
        factory_cost_raw: 0,
        efficiency_raw: 0,
        civilian_factories: 0,
        convoys_assigned: 0,
        convoys_required: 0,
    };
    let err = encode::encode_classic_deals(DealVersion::Legacy, &[21, 22], &[], &[row])
        .expect_err("a wide identity does not fit a packed key");
    assert!(matches!(
        err,
        encode::EncodeError::PackedIdentityOverflow { row: 0 }
    ));
}

#[test]
fn the_encoder_refuses_rows_that_do_not_fit_their_own_header() {
    let values = [1i64, 2];
    let rows = [encode::FlowRowInput {
        tag_db_id: 21,
        flow_kind: 1,
        values_raw: &values,
    }];
    let err = encode::encode_resource_flow(FlowVersion::Variable, 3, 1, &rows, TEST_LIMITS)
        .expect_err("a two-value row in a three-resource trailer");
    assert!(matches!(
        err,
        encode::EncodeError::FlowRowWidth {
            row: 0,
            values: 2,
            expected: 3
        }
    ));

    let err = encode::encode_resource_rows(
        ResourceRowVersion::Identity,
        3,
        0,
        &[b"oil".as_slice()],
        &[],
        TEST_LIMITS,
    )
    .expect_err("one token for three resources");
    assert!(matches!(
        err,
        encode::EncodeError::IdentityCount {
            entries: 1,
            expected: 3
        }
    ));
}

#[test]
fn everything_the_encoder_writes_walks_to_exact_consumption() {
    let core = encode::encode_core(
        7,
        encode::CoreCounts {
            changed_rows: 0,
            status_flags: 0,
            observed_contract_rows: 0,
            emitted_contract_rows: 0,
            buyer_ptr_valid_rows: 0,
            variant_valid_rows: 0,
            amount_valid_rows: 0,
            partial_field_rows: 0,
        },
        &[],
    )
    .expect("core");
    let mut payload = core.clone();
    payload.extend(
        encode::encode_resource_rows(
            ResourceRowVersion::Identity,
            2,
            0,
            &[b"oil".as_slice(), b"steel".as_slice()],
            &[],
            TEST_LIMITS,
        )
        .expect("identity-only MTRR"),
    );
    payload.extend(encode::encode_activation(0b11, 21, 9, 1, 0x015e_c520));
    let seen = walk_all(&payload).expect("walks");
    assert_eq!(
        seen.iter().map(|(t, _, _)| *t).collect::<Vec<_>>(),
        vec![Trailer::ResourceRow, Trailer::Activation]
    );
}

#[test]
fn the_deal_identity_is_read_at_the_width_its_version_spells() {
    let mut b = core_bytes(&[]);
    mtrd(&mut b, DealVersion::WideIdentity, &[21, 22], &[22]);
    let payload = b.done();
    let header = read_core_header(&payload).expect("header");
    let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    cursor.next(&payload).expect("magic");
    let TrailerBody::ClassicDeal(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body") else {
        panic!("expected MTRD");
    };
    let mut rows = c
        .witnesses()
        .walk_to_end_for_test()
        .next
        .incomplete_owners()
        .walk_to_end_for_test()
        .next
        .rows();
    let RowStep::Row { at, .. } = rows.next() else {
        panic!("expected a row");
    };
    let wide = deal_row_at(&payload, at, DealVersion::WideIdentity);
    assert_eq!(wide.identity_exporter_tag_db_id, 21);
    assert_eq!(wide.identity_importer_tag_db_id, 22);
    assert_eq!(wide.identity_ordinal, 1);
    assert_eq!(wide.packed_identity, None);
    assert_eq!(wide.reserved_ordinal_word, None);

    let packed = deal_row_at(&payload, at, DealVersion::PerOwner);
    assert!(packed.packed_identity.is_some());
    assert_ne!(
        (
            packed.identity_exporter_tag_db_id,
            packed.identity_importer_tag_db_id
        ),
        (
            wide.identity_exporter_tag_db_id,
            wide.identity_importer_tag_db_id
        )
    );
}

#[test]
fn an_identity_only_mtrr_delimits_its_identities_and_no_rows() {
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::Identity,
        3,
        &["oil", "steel", "rubber"],
        &[],
        &[],
    );
    let payload = b.done();
    let header = read_core_header(&payload).expect("header");
    let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    cursor.next(&payload).expect("magic");
    let TrailerBody::ResourceRow(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body") else {
        panic!("expected MTRR");
    };
    let t = *c.header();
    assert_eq!(t.row_count, 0);
    assert_eq!(t.identity_len, 3 * TEST_LIMITS.identity_stride());
    let mut identities = c.identities();
    let mut tokens = Vec::new();
    while let RowStep::Row { at, .. } = identities.next() {
        let entry = identity_entry_at(&payload, at, t.identity_stride)
            .expect("the cursor yielded this offset");
        tokens.push(entry.token.expect("token fits the stride").to_vec());
        assert!(entry.padding.expect("padding").iter().all(|b| *b == 0));
    }
    assert_eq!(
        tokens,
        vec![b"oil".to_vec(), b"steel".to_vec(), b"rubber".to_vec()]
    );
    assert_eq!(t.end, payload.len());
    let rows = identities.into_end().expect("identities").next.rows();
    assert_eq!(rows.remaining(), 0);
    assert_eq!(rows.into_end().expect("rows").next.end(), payload.len());
}

#[test]
fn an_mtrf_v3_with_a_non_seven_resource_count_walks() {
    for count in [1u32, 3, 12, TEST_LIMITS.max_resource_count()] {
        let mut b = core_bytes(&[]);
        mtrf(&mut b, FlowVersion::Variable, count, &[1, 2, 3, 4, 5, 6]);
        let payload = b.done();
        let seen = walk_all(&payload).unwrap_or_else(|v| panic!("count {count}: {v:?}"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].2, payload.len());
    }
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtd1_bytes() {
    let mut b = Bytes::new();
    mtd1_v1(&mut b);
    let rows: Vec<DetailRow> = [(1u32, 0b0000011111u32), (2, 0b1111100000)]
        .into_iter()
        .map(|(contract_id, flags)| DetailRow {
            contract_id,
            flags,
            price_raw: 100_000,
            amount_raw: 200_000,
            speed_raw: 3,
            completed_raw: 4,
            days: 5,
            delivery_country_tag_db_id: 12,
            convoys_assigned: 6,
            convoys_total: 7,
            efficiency_raw: 8,
            factory_cic_progress_raw: 9,
            equipment_transfer_progress_raw: 10,
            reserved: 0,
        })
        .collect();
    assert_eq!(
        encode::encode_detail(
            &rows,
            encode::DetailCounts {
                price_valid_rows: 1,
                speed_valid_rows: 1,
                delivery_route_valid_rows: 1,
                progress_valid_rows: 1,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

#[test]
fn the_mtd1_encoder_puts_each_counted_word_in_its_own_slot() {
    let base = encode::encode_detail(
        &[],
        encode::DetailCounts {
            price_valid_rows: 1,
            speed_valid_rows: 2,
            delivery_route_valid_rows: 3,
            progress_valid_rows: 4,
        },
    )
    .expect("encodes");
    assert_eq!(&base[16..20], &1u32.to_le_bytes());
    assert_eq!(&base[20..24], &2u32.to_le_bytes());
    assert_eq!(&base[24..28], &3u32.to_le_bytes());
    assert_eq!(&base[28..32], &4u32.to_le_bytes());
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrr_bytes_at_every_version() {
    for (version, resource_count, tokens, countries, rows) in [
        (
            ResourceRowVersion::Legacy,
            7u32,
            Vec::<&str>::new(),
            vec![21u32],
            (0..7u32)
                .map(|i| {
                    if i == 0 {
                        (i, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL)
                    } else {
                        (i, NeededType::Surplus.wire(), 0)
                    }
                })
                .collect::<Vec<_>>(),
        ),
        (
            ResourceRowVersion::CountOnly,
            3,
            Vec::new(),
            vec![21],
            vec![
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
                (2, NeededType::Deficit.wire(), 0),
            ],
        ),
        (
            ResourceRowVersion::Identity,
            3,
            vec!["oil", "steel", "rubber"],
            vec![21, 22],
            vec![
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
                (2, NeededType::Deficit.wire(), 0),
            ],
        ),
        (
            ResourceRowVersion::NeededUnavailable,
            2,
            vec!["oil", "steel"],
            vec![21],
            vec![
                (
                    0,
                    NeededType::Unavailable.wire(),
                    ResourceFlag::NeededUnavailable.bit(),
                ),
                (1, NeededType::Surplus.wire(), 0),
            ],
        ),
    ] {
        let mut b = Bytes::new();
        mtrr(&mut b, version, resource_count, &tokens, &countries, &rows);
        let mut encoded_rows = Vec::new();
        for country in &countries {
            for (resource_index, needed_type, flags) in &rows {
                encoded_rows.push(ResourceRow {
                    tag_db_id: *country,
                    resource_index: *resource_index,
                    visible_order: *resource_index,
                    needed_type: *needed_type,
                    balance: 5,
                    produced: 5,
                    imported: 0,
                    exported: 0,
                    special_project: 0,
                    production_line: 0,
                    needed: 5,
                    needed_current: 0,
                    needed_max: 0,
                    flags: *flags,
                    descriptor_db_index: resource_index + 1,
                    reserved: 0,
                });
            }
        }
        let token_bytes: Vec<&[u8]> = tokens.iter().map(|t| t.as_bytes()).collect();
        assert_eq!(
            encode::encode_resource_rows(
                version,
                resource_count,
                countries.len() as u32,
                &token_bytes,
                &encoded_rows,
                TEST_LIMITS,
            )
            .expect("encodes"),
            b.done(),
            "MTRR {version:?}"
        );
    }
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrd_bytes_at_every_version() {
    for version in DealVersion::ALL {
        let incomplete: Vec<u32> = if version.carries_incomplete_owners() {
            vec![22]
        } else {
            Vec::new()
        };
        let mut b = Bytes::new();
        mtrd(&mut b, version, &[21, 22], &incomplete);
        let mut rows = Vec::new();
        for (owner, counterpart, direction, route, status) in [
            (
                21u32,
                22u32,
                DealDirection::Export.wire(),
                DealRoute::Sea.wire(),
                DealStatus::NoTradeInfluence.bit(),
            ),
            (
                22,
                21,
                DealDirection::Import.wire(),
                DealRoute::Land.wire(),
                DealStatus::NoResources.bit(),
            ),
        ] {
            rows.push(DealRow {
                owner_tag_db_id: owner,
                counterpart_tag_db_id: counterpart,
                resource_index: 1,
                visible_order: 0,
                direction,
                route_kind: route,
                flags: DEAL_FLAG_EXACT,
                route_status_or_reserved: if version.carries_route_status() {
                    status
                } else {
                    0
                },
                identity_exporter_tag_db_id: 21,
                identity_importer_tag_db_id: 22,
                identity_ordinal: 1,
                packed_identity: None,
                reserved_ordinal_word: None,
                requested_raw: FIXED_SCALE_RAW as i64,
                delivered_raw: 0,
                available_export_raw: 0,
                factory_cost_raw: 0,
                efficiency_raw: 0,
                civilian_factories: 0,
                convoys_assigned: 0,
                convoys_required: 0,
            });
        }
        assert_eq!(
            encode::encode_classic_deals(version, &[21, 22], &incomplete, &rows).expect("encodes"),
            b.done(),
            "MTRD {version:?}"
        );
    }
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtrb_bytes() {
    let mut b = Bytes::new();
    mtrb_v1(&mut b);
    let row = ResidualRow {
        gap_id: 0,
        status: 1,
        evidence_mask: 2,
        current_support: 3,
        current_domain_id: 4,
        proposed_record_kind: 5,
        current_row_len: 6,
        source_ref_id: 7,
        owner_boundary: 8,
        root_status: 9,
        proof_status: 10,
        reserved: 11,
    };
    assert_eq!(
        encode::encode_residual(
            &[row],
            encode::ResidualCounts {
                source_blocked_rows: 1,
                schema_blocked_rows: 0,
                owned_elsewhere_rows: 0,
                shared_patch_blocked_rows: 0,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

#[test]
fn the_encoder_reproduces_the_handwritten_mtll_bytes() {
    let mut b = Bytes::new();
    mtll_v1(&mut b);
    let row = LendLeaseRow {
        contract_id: 1,
        perspective_tag_db_id: 11,
        counterpart_tag_db_id: 12,
        direction: LendLeaseDirection::Outgoing.wire(),
        equipment_variant_id: 100,
        flags: LendLeaseFlag::AmountValid.bit(),
        amount_raw: FIXED_SCALE_RAW as i64,
        price_raw: 0,
        source_record_kind: 0,
        reserved: 0,
    };
    assert_eq!(
        encode::encode_lend_lease(
            &[row],
            encode::LendLeaseCounts {
                outgoing_rows: 1,
                incoming_rows: 0,
                amount_valid_rows: 1,
                price_valid_rows: 0,
            },
        )
        .expect("encodes"),
        b.done()
    );
}

#[test]
fn every_family_has_an_encoder_golden() {
    let with_goldens = [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::ResourceRow,
        Trailer::ClassicDeal,
        Trailer::Activation,
        Trailer::Stockpile,
        Trailer::Residual,
        Trailer::LendLease,
    ];
    for trailer in Trailer::ALL {
        assert!(
            with_goldens.contains(&trailer),
            "{} has no encoder-to-handwritten-bytes test",
            trailer.label()
        );
    }
}

#[test]
fn the_lane_only_families_have_row_readers() {
    let mut b = core_bytes(&[]);
    mtrb_v1(&mut b);
    mtll_v1(&mut b);
    let payload = b.done();
    let mut saw_residual = false;
    let mut saw_lend_lease = false;
    visit_trailers(&payload, |payload, _, body| match body {
        TrailerBody::Residual(c) => {
            let mut rows = c.rows();
            let RowStep::Row { at, .. } = rows.next() else {
                panic!("expected a row")
            };
            let row = residual_row_at(payload, at);
            assert_eq!(row.gap_id, 0);
            assert_eq!(row.proof_status, 10);
            assert_eq!(row.reserved, 11);
            saw_residual = true;
            Ok(rows.walk_to_end_for_test().next)
        }
        TrailerBody::LendLease(c) => {
            let mut rows = c.rows();
            let RowStep::Row { at, .. } = rows.next() else {
                panic!("expected a row")
            };
            let row = lend_lease_row_at(payload, at);
            assert_eq!(row.contract_id, 1);
            assert_eq!(row.direction, LendLeaseDirection::Outgoing.wire());
            assert_eq!(row.amount_raw, FIXED_SCALE_RAW as i64);
            assert_eq!(row.reserved, 0);
            saw_lend_lease = true;
            Ok(rows.walk_to_end_for_test().next)
        }
        _ => Ok(walk_body_to_end_for_test(body)),
    })
    .expect("walk");
    assert!(saw_residual && saw_lend_lease);
}

#[test]
fn the_flow_masks_are_the_documented_literals() {
    assert_eq!(FLOW_KIND_MASK, 0x3F);
    assert_eq!(FLOW_RESOURCE_MASK, 0x7F);
}

fn a_fixture_with_every_block_filled(trailer: Trailer) -> Vec<u8> {
    let mut b = core_bytes(&[]);
    match trailer {
        Trailer::Detail => mtd1_v1(&mut b),
        Trailer::ResourceFlow => mtrf(&mut b, FlowVersion::Variable, 3, &[1, 2, 3, 4, 5, 6]),
        Trailer::FactoryHeader => mtrh_v1(&mut b),
        Trailer::ResourceRow => mtrr(
            &mut b,
            ResourceRowVersion::Identity,
            2,
            &["oil", "steel"],
            &[21],
            &[
                (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
                (1, NeededType::Surplus.wire(), 0),
            ],
        ),
        Trailer::ClassicDeal => mtrd(&mut b, DealVersion::WideIdentity, &[21, 22], &[22]),
        Trailer::Activation => mtra_v1(&mut b),
        Trailer::Stockpile => mts1_v1(&mut b),
        Trailer::Residual => mtrb_v1(&mut b),
        Trailer::LendLease => mtll_v1(&mut b),
    }
    b.done()
}

fn only_body(payload: &[u8]) -> TrailerBody {
    let header = read_core_header(payload).expect("header");
    let cursor = TrailerCursor::new(core_end(payload, header.row_count()).expect("core"));
    let mut cursor = cursor;
    let TrailerStep::Magic { .. } = cursor.next(payload).expect("magic") else {
        panic!("expected a magic");
    };
    cursor.read_body(payload, TEST_LIMITS).expect("body")
}

fn assert_not_consumed(err: Violation, trailer: Trailer, block: TrailerBlock, remaining: usize) {
    assert_eq!(
        err.kind,
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed {
            trailer,
            block,
            remaining,
        }),
        "{} {} produced {:?}",
        trailer.label(),
        block.name(),
        err.kind
    );
}

#[test]
fn every_declared_block_refuses_to_end_before_it_is_consumed() {
    let mut covered: Vec<(Trailer, TrailerBlock)> = Vec::new();
    let mut note = |trailer: Trailer, block: TrailerBlock| {
        if !covered.contains(&(trailer, block)) {
            covered.push((trailer, block));
        }
    };

    for trailer in [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::Residual,
        Trailer::LendLease,
    ] {
        let payload = a_fixture_with_every_block_filled(trailer);
        let rows = match only_body(&payload) {
            TrailerBody::Detail(c) => c.rows(),
            TrailerBody::ResourceFlow(c) => c.rows(),
            TrailerBody::FactoryHeader(c) => c.rows(),
            TrailerBody::Residual(c) => c.rows(),
            TrailerBody::LendLease(c) => c.rows(),
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        let remaining = rows.remaining();
        assert!(remaining > 0, "{} fixture has no rows", trailer.label());
        let err = rows
            .into_end()
            .unwrap_err_or_panic(&format!("{} rows skipped silently", trailer.label()));
        assert_not_consumed(err, trailer, TrailerBlock::Rows, remaining);
        note(trailer, TrailerBlock::Rows);
    }

    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let identities = c.identities();
    let remaining = identities.remaining();
    assert!(remaining > 0, "MTRR fixture has no identities");
    let err = identities
        .into_end()
        .unwrap_err_or_panic("MTRR identities skipped silently");
    assert_not_consumed(
        err,
        Trailer::ResourceRow,
        TrailerBlock::Identities,
        remaining,
    );
    note(Trailer::ResourceRow, TrailerBlock::Identities);

    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let rows = c.identities().walk_to_end_for_test().next.rows();
    let remaining = rows.remaining();
    assert!(remaining > 0, "MTRR fixture has no rows");
    let err = rows
        .into_end()
        .unwrap_err_or_panic("MTRR rows skipped silently");
    assert_not_consumed(err, Trailer::ResourceRow, TrailerBlock::Rows, remaining);
    note(Trailer::ResourceRow, TrailerBlock::Rows);

    let payload = a_fixture_with_every_block_filled(Trailer::ClassicDeal);
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let witnesses = c.witnesses();
    let remaining = witnesses.remaining();
    assert!(remaining > 0, "MTRD fixture has no witnesses");
    let err = witnesses
        .into_end()
        .unwrap_err_or_panic("MTRD witnesses skipped silently");
    assert_not_consumed(
        err,
        Trailer::ClassicDeal,
        TrailerBlock::Witnesses,
        remaining,
    );
    note(Trailer::ClassicDeal, TrailerBlock::Witnesses);

    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let incomplete = c
        .witnesses()
        .walk_to_end_for_test()
        .next
        .incomplete_owners();
    let remaining = incomplete.remaining();
    assert!(remaining > 0, "MTRD fixture has no incomplete owners");
    let err = incomplete
        .into_end()
        .unwrap_err_or_panic("MTRD incomplete owners skipped silently");
    assert_not_consumed(
        err,
        Trailer::ClassicDeal,
        TrailerBlock::IncompleteOwners,
        remaining,
    );
    note(Trailer::ClassicDeal, TrailerBlock::IncompleteOwners);

    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let rows = c
        .witnesses()
        .walk_to_end_for_test()
        .next
        .incomplete_owners()
        .walk_to_end_for_test()
        .next
        .rows();
    let remaining = rows.remaining();
    assert!(remaining > 0, "MTRD fixture has no rows");
    let err = rows
        .into_end()
        .unwrap_err_or_panic("MTRD rows skipped silently");
    assert_not_consumed(err, Trailer::ClassicDeal, TrailerBlock::Rows, remaining);
    note(Trailer::ClassicDeal, TrailerBlock::Rows);

    let payload = a_fixture_with_every_block_filled(Trailer::Stockpile);
    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let countries = c.countries();
    let remaining = countries.remaining();
    assert!(remaining > 0, "MTS1 fixture has no countries");
    let err = countries
        .into_end()
        .unwrap_err_or_panic("MTS1 countries skipped silently");
    assert_not_consumed(err, Trailer::Stockpile, TrailerBlock::Countries, remaining);
    note(Trailer::Stockpile, TrailerBlock::Countries);

    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let items = c.countries().walk_to_end_for_test().next.items();
    let remaining = items.remaining();
    assert!(remaining > 0, "MTS1 fixture has no items");
    let err = items
        .into_end()
        .unwrap_err_or_panic("MTS1 items skipped silently");
    assert_not_consumed(err, Trailer::Stockpile, TrailerBlock::Items, remaining);
    note(Trailer::Stockpile, TrailerBlock::Items);

    let payload = a_fixture_with_every_block_filled(Trailer::Activation);
    let TrailerBody::Activation(c) = only_body(&payload) else {
        panic!("expected MTRA");
    };
    assert_eq!(c.end().next.end(), payload.len());

    let mut families: Vec<Trailer> = Vec::new();
    for (trailer, _) in &covered {
        if !families.contains(trailer) {
            families.push(*trailer);
        }
    }
    for trailer in Trailer::ALL {
        if trailer == Trailer::Activation {
            continue;
        }
        assert!(
            families.contains(&trailer),
            "{} has declared blocks and none was probed",
            trailer.label()
        );
    }
    for block in TrailerBlock::ALL {
        let must_be_probed = match block {
            TrailerBlock::Rows
            | TrailerBlock::Identities
            | TrailerBlock::Witnesses
            | TrailerBlock::IncompleteOwners
            | TrailerBlock::Countries
            | TrailerBlock::Items => true,
        };
        assert_eq!(
            covered.iter().any(|(_, b)| *b == block),
            must_be_probed,
            "no family's {} block was probed",
            block.name()
        );
    }
}

#[test]
fn walking_a_block_lands_on_exactly_that_blocks_end() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    let identities = c.identities();
    let walked = identities.walk_to_end_for_test();
    assert_eq!(walked.at, header.rows_at);

    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let mut identities = c.identities();
    while let RowStep::Row { .. } = identities.next() {}
    assert_eq!(
        identities.into_end().expect("identities").at,
        header.rows_at
    );
}

#[test]
fn reading_a_body_that_was_never_delimited_is_a_caller_defect() {
    let payload = a_fixture_with_every_block_filled(Trailer::Detail);
    let header = read_core_header(&payload).expect("header");
    let cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
    let err = cursor
        .read_body(&payload, TEST_LIMITS)
        .unwrap_err_or_panic("an undelimited body was read");
    assert_eq!(
        err.kind,
        ViolationKind::CallerDefect(CallerDefect::BodyNotDelimited)
    );
}

#[test]
fn peeking_a_header_reads_the_bodys_words_and_moves_nothing() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceFlow);
    let header = read_core_header(&payload).expect("header");
    let core = core_end(&payload, header.row_count()).expect("core");
    let mut cursor = TrailerCursor::new(core);
    let TrailerStep::Magic { at, trailer } = cursor.next(&payload).expect("magic") else {
        panic!("expected a magic");
    };
    assert_eq!(trailer, Trailer::ResourceFlow);

    let HeaderPreview::ResourceFlow(words) = cursor.peek_header(&payload).expect("preview") else {
        panic!("expected an MTRF preview");
    };
    let second = cursor.peek_header(&payload).expect("preview");
    assert_eq!(second, HeaderPreview::ResourceFlow(words));
    assert_eq!(cursor.offset(), core);
    assert_eq!(words.sized.prefix.at, at);

    let TrailerBody::ResourceFlow(c) = cursor.read_body(&payload, TEST_LIMITS).expect("body")
    else {
        panic!("expected MTRF");
    };
    let body = *c.header();
    assert_eq!(words.sized.prefix.version, body.version.wire());
    assert_eq!(words.sized.prefix.header_len as usize, body.header_len);
    assert_eq!(words.sized.row_len as usize, body.row_len);
    assert_eq!(words.sized.row_count as usize, body.row_count);
    assert_eq!(
        words.country_count_or_exported as usize,
        body.country_count_or_exported
    );
    assert_eq!(words.scale_raw, body.scale_raw);
    assert_eq!(words.mask_or_count, body.mask_or_count);
    assert_eq!(words.kind_mask, body.kind_mask);
}

#[test]
fn peeking_a_truncated_header_reports_what_reading_it_would() {
    let payload = a_fixture_with_every_block_filled(Trailer::Detail);
    let header = read_core_header(&payload).expect("header");
    let core = core_end(&payload, header.row_count()).expect("core");
    let truncated = &payload[..core + 8];
    let mut cursor = TrailerCursor::new(core);
    cursor.next(truncated).expect("magic");
    let peeked = cursor
        .peek_header(truncated)
        .unwrap_err_or_panic("a truncated header previewed");
    let mut cursor = TrailerCursor::new(core);
    cursor.next(truncated).expect("magic");
    let read = cursor
        .read_body(truncated, TEST_LIMITS)
        .unwrap_err_or_panic("a truncated header read");
    assert_eq!(peeked, read);
    assert_eq!(
        read.kind,
        ViolationKind::Need {
            section: Section::TrailerHeader(Trailer::Detail),
            needed: core + Trailer::Detail.header_len(),
            available: truncated.len(),
        }
    );
}

#[test]
fn completing_a_partly_walked_block_still_lands_on_its_own_end() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    let mut identities = c.identities();
    assert!(
        identities.remaining() >= 2,
        "this case needs a block with at least two rows to be able to fail"
    );
    let RowStep::Row { index, .. } = identities.next() else {
        panic!("expected a row");
    };
    assert_eq!(index, 0);
    assert_eq!(identities.remaining(), 1);
    let walked = identities.walk_to_end_for_test();
    assert_eq!(walked.at, header.rows_at);

    for consume in 0..=2usize {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut identities = c.identities();
        for _ in 0..consume {
            identities.next();
        }
        assert_eq!(
            identities.walk_to_end_for_test().at,
            header.rows_at,
            "after {consume}"
        );
    }
}

#[test]
fn walking_a_version_absent_block_moves_nothing() {
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        2,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    let payload = b.done();
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.identity_len, 0);
    let identities = c.identities();
    assert_eq!(identities.remaining(), 0);
    let walked = identities.walk_to_end_for_test();
    assert_eq!(walked.at, header.identity_at);
    assert_eq!(walked.at, header.rows_at);

    let mut b = core_bytes(&[]);
    mtrd(&mut b, DealVersion::Statuses, &[21, 22], &[]);
    let payload = b.done();
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let header = *c.header();
    assert_eq!(header.incomplete_count, 0);
    let incomplete = c
        .witnesses()
        .walk_to_end_for_test()
        .next
        .incomplete_owners();
    assert_eq!(incomplete.remaining(), 0);
    assert_eq!(incomplete.walk_to_end_for_test().at, header.rows_at);
}

#[test]
fn peeking_reports_a_header_the_body_refuses() {
    for (version, declared_header_len, declared_row_len) in [
        (1u16, 64u16, 64u16),
        (9u16, 32u16, 64u16),
        (1u16, 32u16, 48u16),
    ] {
        let mut b = core_bytes(&[]);
        b.magic(b"MTD1")
            .u16(version)
            .u16(declared_header_len)
            .u16(declared_row_len)
            .u16(0)
            .u32(0);
        b.zeros(16);
        let payload = b.done();
        let header = read_core_header(&payload).expect("header");
        let core = core_end(&payload, header.row_count()).expect("core");
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");

        let HeaderPreview::Detail(words) = cursor
            .peek_header(&payload)
            .expect("the preview applies no rule, so a header the body refuses still previews")
        else {
            panic!("expected an MTD1 preview");
        };
        assert_eq!(words.sized.prefix.version, version);
        assert_eq!(words.sized.prefix.header_len, declared_header_len);
        assert_eq!(words.sized.row_len, declared_row_len);
        assert_eq!(cursor.offset(), core);

        cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic("a malformed MTD1 header was accepted");
    }
}

#[test]
fn every_family_previews_inside_its_own_header() {
    for trailer in Trailer::ALL {
        let payload = a_fixture_with_every_block_filled(trailer);
        let header = read_core_header(&payload).expect("header");
        let core = core_end(&payload, header.row_count()).expect("core");
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor
            .peek_header(&payload)
            .unwrap_or_else(|error| panic!("{} preview: {error:?}", trailer.label()));
        let prefix = preview.prefix();
        assert_eq!(prefix.trailer, trailer, "{}", trailer.label());
        assert_eq!(prefix.at, core, "{}", trailer.label());
        assert_eq!(
            prefix.header_len as usize,
            trailer.header_len(),
            "{} previewed a declared header_len its own table disagrees with",
            trailer.label()
        );
        let short = &payload[..core + trailer.header_len() - 1];
        let mut cursor = TrailerCursor::new(core);
        cursor.next(short).expect("magic");
        cursor
            .peek_header(short)
            .unwrap_err_or_panic(&format!("{} previewed past its header", trailer.label()));
    }
}

#[test]
fn an_identity_stride_an_entry_cannot_fit_is_refused() {
    for stride in [0usize, 1] {
        let limits = Limits::new(TEST_LIMITS.max_resource_count(), stride);
        let mut b = core_bytes(&[]);
        b.magic(b"MTRR")
            .u16(ResourceRowVersion::Identity.wire())
            .u16(32)
            .u16(64)
            .u16(0)
            .u32(0);
        b.u32(0).u32(1).u32(1).u32(stride as u32);
        let payload = b.done();
        let header = read_core_header(&payload).expect("header");
        let mut cursor = TrailerCursor::new(core_end(&payload, header.row_count()).expect("core"));
        cursor.next(&payload).expect("magic");
        let err = cursor
            .read_body(&payload, limits)
            .unwrap_err_or_panic(&format!("a stride of {stride} was accepted"));
        assert_eq!(
            err.kind,
            ViolationKind::IdentityStride {
                trailer: Trailer::ResourceRow,
                stride,
            }
        );
    }
}

#[test]
fn a_version_with_no_identity_block_yields_no_entries() {
    let mut b = core_bytes(&[]);
    mtrr(
        &mut b,
        ResourceRowVersion::CountOnly,
        2,
        &[],
        &[21],
        &[
            (0, NeededType::CurrentMax.wire(), RESOURCE_FLAGS_OIL),
            (1, NeededType::Surplus.wire(), 0),
        ],
    );
    let payload = b.done();
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.identity_stride, 0);
    let identities = c.identities();
    assert_eq!(identities.remaining(), 0);
    assert!(identity_entry_at(&payload, header.identity_at, header.identity_stride).is_none());

    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let header = *c.header();
    assert_eq!(header.resource_count, 2);
    let mut identities = c.identities();
    let mut offsets = Vec::new();
    while let RowStep::Row { at, .. } = identities.next() {
        offsets.push(at);
        assert!(identity_entry_at(&payload, at, header.identity_stride).is_some());
    }
    assert_eq!(offsets.len(), 2);
    assert_eq!(offsets[1] - offsets[0], header.identity_stride);
}

#[test]
fn every_familys_preview_reports_a_header_its_body_refuses() {
    for trailer in Trailer::ALL {
        let good = a_fixture_with_every_block_filled(trailer);
        let header = read_core_header(&good).expect("header");
        let core = core_end(&good, header.row_count()).expect("core");
        let mut payload = good.clone();
        let bad = (trailer.header_len() as u16) + 8;
        payload[core + TRAILER_HEADER_LEN_OFFSET] = (bad & 0xFF) as u8;
        payload[core + TRAILER_HEADER_LEN_OFFSET + 1] = (bad >> 8) as u8;

        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor.peek_header(&payload).unwrap_or_else(|error| {
            panic!(
                "{} previewed a header its body refuses: {error:?}",
                trailer.label()
            )
        });
        assert_eq!(
            preview.prefix().header_len,
            bad,
            "{} previewed a value it was not given",
            trailer.label()
        );
        assert_eq!(cursor.offset(), core, "{}", trailer.label());

        let err = cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic(&format!("{} accepted a wrong header_len", trailer.label()));
        assert_eq!(
            err.kind,
            ViolationKind::HeaderLen {
                trailer,
                declared: bad as usize,
                expected: trailer.header_len(),
            },
            "{}",
            trailer.label()
        );

        let mut payload = good.clone();
        payload[core + TRAILER_VERSION_OFFSET] = 0xFE;
        payload[core + TRAILER_VERSION_OFFSET + 1] = 0x00;
        let mut cursor = TrailerCursor::new(core);
        cursor.next(&payload).expect("magic");
        let preview = cursor.peek_header(&payload).unwrap_or_else(|error| {
            panic!(
                "{} previewed an unknown version: {error:?}",
                trailer.label()
            )
        });
        assert_eq!(preview.prefix().version, 0x00FE, "{}", trailer.label());
        let err = cursor
            .read_body(&payload, TEST_LIMITS)
            .unwrap_err_or_panic(&format!("{} accepted an unknown version", trailer.label()));
        assert!(
            matches!(
                err.kind,
                ViolationKind::UnknownVersion {
                    version: 0x00FE,
                    ..
                }
            ),
            "{} produced {:?}",
            trailer.label(),
            err.kind
        );
    }
}

#[test]
fn every_declared_block_walks_to_its_own_end_from_every_position() {
    for trailer in [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::Residual,
        Trailer::LendLease,
    ] {
        let payload = a_fixture_with_every_block_filled(trailer);
        let total = match only_body(&payload) {
            TrailerBody::Detail(c) => c.rows().remaining(),
            TrailerBody::ResourceFlow(c) => c.rows().remaining(),
            TrailerBody::FactoryHeader(c) => c.rows().remaining(),
            TrailerBody::Residual(c) => c.rows().remaining(),
            TrailerBody::LendLease(c) => c.rows().remaining(),
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        assert!(total > 0, "{} has no rows", trailer.label());
        let declared_end = match only_body(&payload) {
            TrailerBody::Detail(c) => c.header().sized.end,
            TrailerBody::ResourceFlow(c) => c.header().end,
            TrailerBody::FactoryHeader(c) => c.header().sized.end,
            TrailerBody::Residual(c) => c.header().sized.end,
            TrailerBody::LendLease(c) => c.header().sized.end,
            _ => panic!("{} fixture produced another family", trailer.label()),
        };
        assert_eq!(
            declared_end,
            payload.len(),
            "{} fixture is not a single trailer",
            trailer.label()
        );
        for consume in 0..=total {
            let mut rows = match only_body(&payload) {
                TrailerBody::Detail(c) => c.rows(),
                TrailerBody::ResourceFlow(c) => c.rows(),
                TrailerBody::FactoryHeader(c) => c.rows(),
                TrailerBody::Residual(c) => c.rows(),
                TrailerBody::LendLease(c) => c.rows(),
                _ => panic!("{} fixture produced another family", trailer.label()),
            };
            for _ in 0..consume {
                rows.next();
            }
            assert_eq!(
                rows.walk_to_end_for_test().at,
                declared_end,
                "{} rows walked to the wrong end after {consume}",
                trailer.label()
            );
        }
    }

    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let mtrr_header = *c.header();
    for consume in 0..=mtrr_header.resource_count as usize {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut identities = c.identities();
        for _ in 0..consume {
            identities.next();
        }
        assert_eq!(
            identities.walk_to_end_for_test().at,
            mtrr_header.rows_at,
            "MTRR identities after {consume}"
        );
    }
    for consume in 0..=mtrr_header.row_count {
        let TrailerBody::ResourceRow(c) = only_body(&payload) else {
            panic!("expected MTRR");
        };
        let mut rows = c.identities().walk_to_end_for_test().next.rows();
        for _ in 0..consume {
            rows.next();
        }
        assert_eq!(
            rows.walk_to_end_for_test().at,
            mtrr_header.end,
            "MTRR rows after {consume}"
        );
    }

    let payload = a_fixture_with_every_block_filled(Trailer::ClassicDeal);
    let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
        panic!("expected MTRD");
    };
    let mtrd = *c.header();
    for consume in 0..=mtrd.witness_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut witnesses = c.witnesses();
        for _ in 0..consume {
            witnesses.next();
        }
        assert_eq!(
            witnesses.walk_to_end_for_test().at,
            mtrd.incomplete_at,
            "MTRD witnesses after {consume}"
        );
    }
    for consume in 0..=mtrd.incomplete_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut incomplete = c
            .witnesses()
            .walk_to_end_for_test()
            .next
            .incomplete_owners();
        for _ in 0..consume {
            incomplete.next();
        }
        assert_eq!(
            incomplete.walk_to_end_for_test().at,
            mtrd.rows_at,
            "MTRD incomplete after {consume}"
        );
    }
    for consume in 0..=mtrd.row_count {
        let TrailerBody::ClassicDeal(c) = only_body(&payload) else {
            panic!("expected MTRD");
        };
        let mut rows = c
            .witnesses()
            .walk_to_end_for_test()
            .next
            .incomplete_owners()
            .walk_to_end_for_test()
            .next
            .rows();
        for _ in 0..consume {
            rows.next();
        }
        assert_eq!(
            rows.walk_to_end_for_test().at,
            mtrd.end,
            "MTRD rows after {consume}"
        );
    }

    let payload = a_fixture_with_every_block_filled(Trailer::Stockpile);
    let TrailerBody::Stockpile(c) = only_body(&payload) else {
        panic!("expected MTS1");
    };
    let mts1 = *c.header();
    for consume in 0..=mts1.country_count {
        let TrailerBody::Stockpile(c) = only_body(&payload) else {
            panic!("expected MTS1");
        };
        let mut countries = c.countries();
        for _ in 0..consume {
            countries.next();
        }
        assert_eq!(
            countries.walk_to_end_for_test().at,
            mts1.items_at,
            "MTS1 countries after {consume}"
        );
    }
    for consume in 0..=mts1.item_count {
        let TrailerBody::Stockpile(c) = only_body(&payload) else {
            panic!("expected MTS1");
        };
        let mut items = c.countries().walk_to_end_for_test().next.items();
        for _ in 0..consume {
            items.next();
        }
        assert_eq!(
            items.walk_to_end_for_test().at,
            mts1.end,
            "MTS1 items after {consume}"
        );
    }
}

/// is the argument checking, and all of it is RELEASE checking: a
#[test]
fn a_forged_identity_read_is_refused_rather_than_panicking() {
    let payload = a_fixture_with_every_block_filled(Trailer::ResourceRow);
    let TrailerBody::ResourceRow(c) = only_body(&payload) else {
        panic!("expected MTRR");
    };
    let parsed = *c.header();
    let stride = parsed.identity_stride;
    assert!(stride >= 2);
    assert!(identity_entry_at(&payload, parsed.identity_at, stride).is_some());

    for bad in [0usize, 1] {
        assert!(
            identity_entry_at(&payload, parsed.identity_at, bad).is_none(),
            "a stride of {bad} was read"
        );
    }

    assert!(identity_entry_at(&payload, usize::MAX - 1, stride).is_none());
    assert!(identity_entry_at(&payload, parsed.identity_at, usize::MAX).is_none());

    assert!(identity_entry_at(&payload, payload.len(), stride).is_none());
    let short = &payload[..parsed.identity_at + 1];
    assert!(identity_entry_at(short, parsed.identity_at, stride).is_none());

    let exact = &payload[..parsed.identity_at + stride];
    assert!(identity_entry_at(exact, parsed.identity_at, stride).is_some());

    let first = identity_entry_at(&payload, parsed.identity_at, stride).expect("slot 0");
    let second = identity_entry_at(&payload, parsed.identity_at + stride, stride).expect("slot 1");
    assert_ne!(first.token, second.token);
}
