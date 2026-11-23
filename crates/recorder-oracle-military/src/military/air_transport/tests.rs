use super::*;
use replay_data::{rows::AirWingRaidReference, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

const WING: u64 = (u32::MAX as u64) << 32 | 69;
const RAID: u64 = (u32::MAX as u64) << 32 | 84;

fn content() -> SubUnits {
    let mut content = SubUnits::default();
    content.read(
        b"sub_units={ModTransport={type=air_transport} Fighter={type=fighter}}",
        "fixture/units.txt",
    );
    content
}

fn wing() -> AirWingRow {
    AirWingRow {
        id: WING,
        pool_subunit: Recorded::Value(Some(1)),
        raid_reference: Recorded::Value(Some(AirWingRaidReference {
            raw: RAID,
            resolves: true,
        })),
        ..Default::default()
    }
}

fn saved(pool_type: &[u8], raid: Option<u64>) -> save::air::Wing {
    save::air::Wing {
        id: WING,
        pool_type: pool_type.to_vec(),
        name: None,
        accidents: None,
        experience: save::Decimal::parse(b"0").unwrap(),
        equipment: save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid,
    }
}

#[test]
fn native_saved_raid_instance_reaches_full_reference_comparison() {
    let doc = content_reader_parser::parser::parse(
        b"strategic_air={MOD={air_wing_pool={definition=ModTransport \
          air_wings={id={id=4294967295 type=69} experience=0 \
          equipment={allow_zero_entries=no} \
          raid_instance={id=4294967295 type=84}}}}}",
    )
    .unwrap();
    let pack = save::air::parse(&doc.items).unwrap();
    let mut report = Report::default();
    raid(
        &wing(),
        &pack.wings[0],
        Some(true),
        "fixture wing",
        &mut report,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["AirWings.raid_reference"], 1);

    let mut refused = wing();
    refused.air_base = 17;
    refused.raid_reference = None.into();
    let mut report = Report::default();
    raid(
        &refused,
        &pack.wings[0],
        Some(true),
        "fixture wing",
        &mut report,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.limitations[RAID_NOT_RECORDED],
        ["fixture wing.raid_reference"]
    );
}

fn run(
    actual: AirWingRow,
    expected: save::air::Wing,
    content: ContentRead<'_, SubUnits>,
) -> Report {
    run_literals(
        actual,
        expected,
        content,
        &[(100, b"ModTransport"), (101, b"Fighter")],
    )
}

fn run_literals(
    actual: AirWingRow,
    expected: save::air::Wing,
    content: ContentRead<'_, SubUnits>,
    literals: &[(u32, &[u8])],
) -> Report {
    let (mut frame, mut save, _, _) = super::super::super::fixtures::fixture();
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![actual]));
    save.air.wings.push(expected);
    let mut report = Report::default();
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        literals
            .iter()
            .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
                token: *token,
                literal: literal.to_vec(),
            })
            .collect(),
    ));
    let tokens = super::super::super::lexical_tokens(&frame, &mut report);
    compare(&frame, &save, &content, &tokens, &mut report).unwrap();
    report
}

#[test]
fn pool_index_compares_exact_recorded_content_spelling_and_native_null_token() {
    content_reader_script::test_game::enter();
    let content = content();
    let report = run(wing(), saved(b"ModTransport", Some(RAID)), Ok(&content));
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.compared_tables["AirWings.pool_subunit"], 1);
    for pool_type in [b"modtransport".as_slice(), b"Fighter", b""] {
        let report = run(wing(), saved(pool_type, Some(RAID)), Ok(&content));
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("pool_subunit spelling"));
    }
    let mut null = wing();
    null.pool_subunit = Recorded::Value(Some(0));
    null.raid_reference = Recorded::Value(None);
    let report = run(
        null.clone(),
        saved(b"none", None),
        Err("unused content".into()),
    );
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    let report = run(null, saved(b"", None), Ok(&content));
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn native_reverse_spelling_is_exact_even_when_content_declared_another_case() {
    content_reader_script::test_game::enter();
    let content = content();
    let literals = [(100, b"modtransport".as_slice()), (101, b"Fighter")];
    let report = run_literals(
        wing(),
        saved(b"modtransport", Some(RAID)),
        Ok(&content),
        &literals,
    );
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    let report = run_literals(
        wing(),
        saved(b"ModTransport", Some(RAID)),
        Ok(&content),
        &literals,
    );
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("pool_subunit spelling"));
    let report = run_literals(
        wing(),
        saved(b"ModTransport", Some(RAID)),
        Ok(&content),
        &[(100, b"ModTransport\0ignored")],
    );
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
}

#[test]
fn an_unsettled_content_category_refuses_only_a_reached_raid_admission() {
    content_reader_script::test_game::enter();
    let mut content = content();
    content.read(
        b"sub_units={bad={weights={x @variable=1}}}",
        "fixture/unsettled.txt",
    );
    let mut actual = wing();
    actual.raid_reference = Recorded::Value(None);
    let report = run(actual.clone(), saved(b"ModTransport", None), Ok(&content));
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .next()
            .unwrap()
            .contains("@variable")
    );
    actual.mission_mask = 1;
    let report = run(actual, saved(b"ModTransport", None), Ok(&content));
    assert!(report.failed_comparisons.is_empty());
    assert!(report.differences.is_empty());
}

#[test]
fn missing_ambiguous_and_rejected_lexical_identity_refuse_the_pool_join() {
    content_reader_script::test_game::enter();
    let content = content();
    for literals in [
        vec![],
        vec![(100, b"ModTransport".as_slice()), (101, b"modtransport")],
    ] {
        let report = run_literals(
            wing(),
            saved(b"ModTransport", Some(RAID)),
            Ok(&content),
            &literals,
        );
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(
            report
                .failed_comparisons
                .values()
                .next()
                .unwrap()
                .contains("lexical")
        );
    }
    let mut rejected = SubUnits::default();
    rejected.read(
        b"sub_units={123rejected={type=air_transport}}",
        "fixture/rejected.txt",
    );
    let report = run_literals(
        wing(),
        saved(b"123rejected", Some(RAID)),
        Ok(&rejected),
        &[],
    );
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .next()
            .unwrap()
            .contains("rejected lexer token")
    );
}

#[test]
fn identical_native_reverse_aliases_keep_exact_writer_bytes_and_conflicts_stay_ambiguous() {
    content_reader_script::test_game::enter();
    let mut alias_content = SubUnits::default();
    alias_content.read(
        br#"sub_units={">"={type=air_transport}}"#,
        "fixture/reverse-alias.txt",
    );
    let report = run_literals(
        wing(),
        saved(b">", Some(RAID)),
        Ok(&alias_content),
        &[(100, b">"), (20_000, b">\0ignored")],
    );
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());

    let content = content();
    let report = run_literals(
        wing(),
        saved(b"ModTransport", Some(RAID)),
        Ok(&content),
        &[
            (100, b"ModTransport"),
            (101, b"modtransport"),
            (102, b"ModTransport"),
        ],
    );
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .next()
            .unwrap()
            .contains("ambiguous recorded lexical identities")
    );
}

#[test]
fn pool_fault_unknown_index_and_content_failure_do_not_erase_live_raid_differences() {
    content_reader_script::test_game::enter();
    let content = content();
    let mut actual = wing();
    actual.pool_subunit = Recorded::Value(None);
    let report = run(actual, saved(b"ModTransport", Some(RAID)), Ok(&content));
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("pool_subunit"));
    let mut missing_pool = wing();
    missing_pool.air_base = 17;
    missing_pool.pool_subunit = None.into();
    missing_pool.raid_reference = None.into();
    missing_pool.map_flags = Some(16).into();
    let report = run(missing_pool, saved(b"ModTransport", None), Ok(&content));
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("pool_subunit"));
    assert_eq!(report.compared_tables["AirWings.raid_reference"], 1);
    let mut unknown = wing();
    unknown.pool_subunit = Recorded::Value(Some(99));
    let report = run(
        unknown,
        saved(b"ModTransport", Some(RAID ^ (1 << 32))),
        Ok(&content),
    );
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .next()
            .unwrap()
            .contains("index 99")
    );
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("raid_reference.raw"));
    let report = run(
        wing(),
        saved(b"ModTransport", Some(RAID ^ 1)),
        Err("fixture content refused".into()),
    );
    assert_eq!(report.failed_comparisons.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .next()
            .unwrap()
            .contains("fixture content refused")
    );
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn an_unrecorded_admitted_raid_reference_is_a_limitation_whatever_the_save_holds() {
    content_reader_script::test_game::enter();
    let content = content();
    for expected in [Some(RAID), None] {
        let mut actual = wing();
        actual.raid_reference = Recorded::Value(None);
        let report = run(actual, saved(b"ModTransport", expected), Ok(&content));
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            !report
                .compared_tables
                .contains_key("AirWings.raid_reference")
        );
        assert_eq!(report.limitations[RAID_NOT_RECORDED].len(), 1);
    }
}

#[test]
fn pool_backed_active_fighters_compare_certified_nonresolution_without_a_raw_packet() {
    content_reader_script::test_game::enter();
    let content = content();
    for (flags, saved_raid, differences) in [
        (Some(16), None, 0),
        (Some(16), Some(RAID), 1),
        (Some(48), Some(RAID), 1),
        (Some(48), None, 1),
        (Some(0), None, 0),
        (Some(0), Some(RAID), 0),
        (None, Some(RAID), 0),
    ] {
        let mut actual = wing();
        actual.air_base = 17;
        actual.mission_mask = 1;
        actual.pool_subunit = Some(2).into();
        actual.map_flags = flags.into();
        actual.raid_reference = None.into();
        let report = run(actual, saved(b"Fighter", saved_raid), Ok(&content));
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(report.failed_comparisons.is_empty());
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("outside that capture demand"))
        );
        let certified = flags.is_some_and(|flags| flags & 16 != 0);
        assert_eq!(
            report.limitations.contains_key(RAID_NOT_RECORDED),
            !certified
        );
        assert_eq!(
            report
                .compared_tables
                .get("AirWings.raid_reference")
                .copied(),
            certified.then_some(1)
        );
    }
}

#[test]
fn live_raid_compares_both_full_words_and_resolution_in_both_directions() {
    content_reader_script::test_game::enter();
    let content = content();
    for (raw, resolves, expected, differences) in [
        (RAID, true, Some(RAID), 0),
        (RAID ^ (1 << 32), true, Some(RAID), 1),
        (RAID ^ 1, true, Some(RAID), 1),
        (RAID, false, Some(RAID), 1),
        (RAID ^ 1, false, Some(RAID), 2),
        (RAID, true, None, 1),
    ] {
        let mut actual = wing();
        actual.raid_reference = Recorded::Value(Some(AirWingRaidReference { raw, resolves }));
        let report = run(actual, saved(b"ModTransport", expected), Ok(&content));
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(report.failed_comparisons.is_empty());
    }
}

#[test]
fn omitted_null_and_stale_raw_bits_require_an_independent_native_witness() {
    content_reader_script::test_game::enter();
    let content = content();
    for raw in [0, u64::MAX, RAID] {
        let mut actual = wing();
        actual.raid_reference = Recorded::Value(Some(AirWingRaidReference {
            raw,
            resolves: false,
        }));
        let report = run(actual, saved(b"ModTransport", None), Ok(&content));
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("matched independent native read witness"))
        );
    }
}

#[test]
fn intentional_unadmitted_raid_scope_and_older_fields_remain_explicit() {
    content_reader_script::test_game::enter();
    let content = content();
    for gate in 0..4 {
        let mut actual = wing();
        actual.raid_reference = Recorded::Value(None);
        let pool_type = match gate {
            0 => {
                actual.pool_subunit = Recorded::Value(Some(2));
                b"Fighter".as_slice()
            }
            1 => {
                actual.mission_mask = 1;
                b"ModTransport"
            }
            2 => {
                actual.eligibility_veto = 1;
                b"ModTransport"
            }
            _ => {
                actual.eligibility_counter = 1;
                actual.eligibility_limit = 0;
                b"ModTransport"
            }
        };
        let report = run(actual, saved(pool_type, Some(RAID)), Ok(&content));
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("outside that capture demand"))
        );
    }
    let mut old = wing();
    old.pool_subunit = Recorded::NotRecorded;
    old.raid_reference = Recorded::NotRecorded;
    let report = run(
        old,
        saved(b"ModTransport", Some(RAID)),
        Err("unused content".into()),
    );
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.limitations.len(), 2);
}
