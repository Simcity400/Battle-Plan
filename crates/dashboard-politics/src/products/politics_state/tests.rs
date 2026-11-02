use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};

pub(super) fn frame(hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    }
}

pub(super) fn set(frame: &mut DecodedFrame, id: TableId, rows: TableRows) {
    *frame.tables[id.index()] = TableView::Complete(rows);
}

/// Countries `(raw id, slot, tag)`, each linked from its own id; `aliases`
/// link further raw ids to slots.
pub(super) fn countries(frame: &mut DecodedFrame, rows: &[(u32, u32, &str)], aliases: &[(u32, u32)]) {
    set(
        frame,
        TableId::Countries,
        TableRows::Countries(
            rows.iter()
                .map(|&(id, slot, tag)| CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(
            rows.iter()
                .map(|&(id, slot, _)| (id, slot))
                .chain(aliases.iter().copied())
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect(),
        ),
    );
}

fn params(view: PoliticsView) -> PoliticsStateParams {
    PoliticsStateParams {
        branch_id: "1".into(),
        hour: 100,
        view,
    }
}

#[test]
fn exiles_cover_the_world_and_keep_unknown_hosts_counted() {
    let mut frame = frame(100);
    countries(
        &mut frame,
        &[(1, 1, "EXA"), (2, 2, "HSA"), (3, 3, "EXB"), (4, 4, "NOX")],
        &[(21, 2)],
    );
    set(
        &mut frame,
        TableId::CountryStabilityInputs,
        TableRows::CountryStabilityInputs(vec![
            CountryStabilityInputRow {
                country: 1,
                exile_host: Some(21).into(),
                exile_legitimacy: Some(45_000).into(),
                exile_manpower: Some(0).into(),
                exile_manpower_cap: Some(-5).into(),
                exile_leaders: Some(0).into(),
                ..Default::default()
            },
            CountryStabilityInputRow {
                country: 3,
                exile_host: Some(2).into(),
                exile_legitimacy: Recorded::NotRecorded,
                exile_manpower: Some(12_400).into(),
                exile_manpower_cap: Some(24_000).into(),
                exile_leaders: Some(2).into(),
                ..Default::default()
            },
            // A known nonpositive host is not an exile.
            CountryStabilityInputRow {
                country: 4,
                exile_host: Some(0).into(),
                ..Default::default()
            },
            CountryStabilityInputRow {
                country: 2,
                exile_host: Recorded::NotRecorded,
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableId::SliceReads,
        TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::CountryStabilityInputs.index() as u32,
            owner: 1,
            sample_hour: 85,
        }]),
    );
    let reply = read(&frame, &params(PoliticsView::Exiles), None).unwrap();
    assert_eq!(reply.exiles.len(), 2);
    let first = &reply.exiles[0];
    assert_eq!(first.tag, "EXA");
    assert_eq!(first.host.as_deref(), Some("HSA"));
    // Known zero and negative values stay exact.
    assert_eq!((first.manpower, first.cap, first.leaders), (Some(0), Some(-5), Some(0)));
    assert_eq!(first.legitimacy.as_deref(), Some("45000"));
    assert_eq!(first.read_hour, Some(85));
    assert_eq!(reply.exiles[1].legitimacy, None);
    for (label, count) in [("exile host roster", 1), ("exile legitimacy", 1)] {
        assert!(
            reply
                .unavailable
                .iter()
                .any(|entry| entry.label == label && entry.count == count),
            "{label}: {:?}",
            reply.unavailable
        );
    }
}

pub(super) fn balance(ordinal: u32, token: &str, value: i64) -> PowerBalanceRow {
    PowerBalanceRow {
        ordinal,
        value_raw: value,
        balance: token.as_bytes().to_vec(),
        left: b"mod_left".to_vec(),
        right: b"mod_right".to_vec(),
        ..Default::default()
    }
}

pub(super) fn member(record: u32, ordinal: u32, country: u32) -> PowerBalanceMemberRow {
    PowerBalanceMemberRow {
        record,
        ordinal,
        country,
        ..Default::default()
    }
}

#[test]
fn balances_keep_stored_order_shared_records_and_exact_member_values() {
    let mut frame = frame(100);
    countries(&mut frame, &[(1, 1, "BPA"), (2, 2, "BPB")], &[(12, 2)]);
    set(
        &mut frame,
        TableId::PowerBalances,
        TableRows::PowerBalances(vec![
            PowerBalanceRow {
                active_range: b"zz_mod_range".to_vec(),
                ..balance(0, "zz_mod_balance", -34_000)
            },
            balance(1, "aa_mod_balance", 100_000),
        ]),
    );
    set(
        &mut frame,
        TableId::PowerBalanceMembers,
        TableRows::PowerBalanceMembers(vec![
            // BPA is in both records: the stored first one is shown, not the
            // first by name.
            member(1, 0, 1),
            PowerBalanceMemberRow {
                daily_raw: 150_000,
                daily_present: true,
                daily_flags: engine_native::modifier_definition::CLAMP_MAXIMUM,
                weekly_raw: -2_500,
                weekly_present: true,
                ..member(0, 0, 1)
            },
            // A shared record, its second member through an alias.
            member(0, 1, 12),
        ]),
    );
    let reply = read(&frame, &params(PoliticsView::Balances), None).unwrap();
    assert_eq!(
        reply.current,
        Some(vec![
            CurrentBalance {
                tag: "BPA".into(),
                record: 0
            },
            CurrentBalance {
                tag: "BPB".into(),
                record: 0
            },
        ])
    );
    let first = &reply.balances[0];
    assert_eq!(first.value, "-34000");
    assert_eq!(first.members.len(), 2);
    // The getter clamps flagged values to the fixed5 unit.
    assert_eq!(first.members[0].daily.as_deref(), Some("100000"));
    assert_eq!(first.members[0].weekly.as_deref(), Some("-2500"));
    assert_eq!(first.members[1].tag.as_deref(), Some("BPB"));
    assert_eq!(first.members[1].daily, None);
    // Without content the names are unknown but every number stays.
    assert_eq!(first.name, None);
    assert_eq!(first.sources, None);
    // The active range keeps its token; its bounds are unknown, not refused as unauthored.
    assert_eq!(
        first.range,
        Some(BalanceRange {
            token: "zz_mod_range".into(),
            name: None,
            min: None,
            max: None,
        })
    );
    assert!(reply.unavailable.iter().all(|entry| entry.label != "balance range bounds"));

    // One unresolved member leaves every country's shown balance unknown.
    set(
        &mut frame,
        TableId::PowerBalanceMembers,
        TableRows::PowerBalanceMembers(vec![member(0, 0, 1), member(1, 0, 99)]),
    );
    let reply = read(&frame, &params(PoliticsView::Balances), None).unwrap();
    assert_eq!(reply.current, None);
    assert_eq!(reply.balances.len(), 2);
    assert_eq!(reply.balances[1].members[0].tag, None);
}

#[test]
fn balances_without_the_tables_refuse_by_name() {
    let mut frame = frame(100);
    countries(&mut frame, &[(1, 1, "BPA")], &[]);
    *frame.tables[TableId::PowerBalances.index()] = TableView::Unavailable { reason: "not recorded".into() };
    let reply = read(&frame, &params(PoliticsView::Balances), None).unwrap();
    assert!(reply.balances.is_empty());
    assert_eq!(reply.current, None);
    assert_eq!(reply.unavailable[0].label, "balances of power");
}

#[test]
fn a_table_left_to_the_native_capture_is_read_from_its_index_never_refused_as_unrecorded() {
    use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records, source};
    use replay::container::Replay;
    use replay_data::scope::CaptureMode;
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let asked = PoliticsStateParams {
        branch_id: "1".into(),
        hour: 1,
        view: PoliticsView::Exiles,
    };
    let load = |frame: DecodedFrame| {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let (source, root) = file_source(replay);
        let reply = PoliticsState::load(&source, &asked, &context, &cancelled);
        drop(source);
        std::fs::remove_dir_all(root).ok();
        reply
    };
    // A row frame's own refusal stands, read without the index.
    let row = load(source(1, 1, Some(1))).unwrap();
    assert_eq!(row.unavailable[0].label, "exile status");
    let mut cut = source(1, 1, Some(1));
    // Full leaves DivisionModelInputs out, so a frame in its mode marks it so.
    for table in [TableId::CountryStabilityInputs, TableId::DivisionModelInputs] {
        *cut.tables[table.index()] = TableView::NotRecorded {
            mode: CaptureMode::Full,
        };
    }
    assert_eq!(
        load(cut).unwrap_err().to_string(),
        "The recording holds [CountryStabilityInputs] in its native capture, whose change-run index is not prepared yet"
    );
}
