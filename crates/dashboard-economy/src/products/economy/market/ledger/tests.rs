use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records, source};
use replay_data::rows::{LexicalTokenRow, MarketContractTermsRow, MarketEquipment, MarketPool, TableRows, VariantDefinitionRow};
use replay_frame::frame::{DecodedFrame, TableView};

fn opened(id: &str, frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn ask(hour: i32) -> MarketLedgerTailParams {
    MarketLedgerTailParams { branch_id: "1".into(), hour: Some(hour), from: None, to: None, countries: None, group: None, search: None, sort: None, offset: None, limit: 64, statuses: None }
}

fn event(id: u64, hour: i32, kind: u8, more: MarketEventRow) -> MarketEventRow {
    MarketEventRow { id, occurrence: id, hour, country: 9001, receiver: 44, kind, agreement: 0x12, ..more }
}

fn load(src: &Source, params: &MarketLedgerTailParams) -> MarketLedgerTailReply {
    MarketLedgerTail::load(src, params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_contract_is_one_row_with_its_measures_at_the_hour_and_quotes_labelled_per_variant() {
    let (cat, _folder) = dashboard_base::catalogue::tests::localisation_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }"),
        ("localisation/mod_l_english.yml", "\u{feff}l_english:\n rifle_0:0 \"Default Rifle\"\n"),
    ]);
    let context: names::Context = Ok(names::Content::from_catalogue(Arc::new(cat)));
    let frames: Vec<_> = (100..=112)
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"rifle_0".to_vec() }]));
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow { id: 6, equipment_token: 1, display_name: b"Infantry Equipment I".to_vec(), ..Default::default() }]));
            if (100..=108).contains(&hour) {
                *frame.tables[TableId::MarketContractTerms.index()] = TableView::Complete(TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                    country: 9001,
                    buyer: 44,
                    id: 0x12,
                    kind: 1,
                    equipment: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 500_000 }] }.into(),
                    prices: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 96_000 }] }.into(),
                    ..Default::default()
                }]));
            }
            let events = match hour {
                100 => vec![event(1, hour, 11, MarketEventRow::default())],
                102 => vec![event(2, hour, 21, MarketEventRow { amount_raw: 48_000, ..Default::default() })],
                104 => vec![event(3, hour, 20, MarketEventRow { variant: 6, amount_raw: 250_000, ..Default::default() })],
                106 => vec![event(4, hour, 21, MarketEventRow { amount_raw: 48_000, ..Default::default() })],
                108 => vec![event(5, hour, 20, MarketEventRow { variant: 6, amount_raw: 250_000, ..Default::default() }), event(6, hour, 12, MarketEventRow::default())],
                _ => Vec::new(),
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let src = opened("mkt-contracts", frames);
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let mut prepared = prepare(&src, &query, 1, &LoadContext { content: None }, &ask(105), &AtomicBool::new(false)).unwrap();
    prepared.context = Arc::new(context);
    prepared.namings = Arc::new(namings(&src, &query).unwrap());
    let mid = load_tail_at(&prepared, &query, &ask(105), 105, &AtomicBool::new(false)).unwrap();
    assert_eq!(mid.total, "1");
    let contract = &mid.rows[0];
    assert_eq!((contract.status, contract.live, contract.started_hour, contract.request), (MarketContractStatus::Active, true, Some(100), false));
    assert_eq!(mid.counts[0].total, "1", "the active chip counts the matching population");
    assert_eq!(contract.paid_raw.as_deref(), Some("48000"));
    assert_eq!(contract.quotes, [MarketQuote { variant: "6".into(), name: Some("Infantry Equipment I".into()), name_missing: None, price_raw: "96000".into() }], "a quote is labelled with its variant, never summed");
    assert_eq!((contract.items[0].delivered_raw.as_deref(), contract.items[0].contracted_raw.as_deref()), (Some("250000"), Some("500000")));
    assert_eq!(contract.events.iter().map(|ev| (ev.hour, ev.kind)).collect::<Vec<_>>(), [(104, MarketContractEventKind::Delivery), (102, MarketContractEventKind::Payment), (100, MarketContractEventKind::ContractStarted)]);
    assert_eq!(mid.next_change, Some(106), "the next payment is the next change");
    let done = load_tail_at(&prepared, &query, &ask(110), 110, &AtomicBool::new(false)).unwrap();
    assert_eq!((done.rows[0].status, done.rows[0].ended_hour, done.rows[0].paid_raw.as_deref()), (MarketContractStatus::Ended, Some(108), Some("96000")));
    assert_eq!(done.rows[0].events_total, "6");
    let ended_only = load(&src, &MarketLedgerTailParams { statuses: Some(vec![MarketContractStatus::Active]), ..ask(110) });
    assert_eq!(ended_only.total, "0", "the status filter applies at the hour asked, before counting");
    let asked = serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "limit": 64, "countries": ["GER"] });
    let panel = dated_panel(&src, &LoadContext { content: None }, &asked).unwrap();
    let exact = panel.exact().unwrap();
    for hour in [100, 102, 105, 108, 104, 112] {
        let stepped = (exact.advance)(hour, true, &AtomicBool::new(false), None);
        assert_eq!(*stepped.value, (exact.cold)(hour, &AtomicBool::new(false)), "stepped vs cold at {hour}");
    }
}

#[test]
fn a_request_and_its_accepted_contract_are_two_rows_and_the_request_removal_ends_only_the_request() {
    let frames: Vec<_> = (100..=106)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let events = match hour {
                100 => vec![event(1, hour, 10, MarketEventRow::default())],
                103 => vec![event(2, hour, 11, MarketEventRow::default()), event(3, hour, 12, MarketEventRow::default())],
                104 => vec![event(4, hour, 21, MarketEventRow { amount_raw: 7_000, ..Default::default() })],
                _ => Vec::new(),
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let src = opened("mkt-request", frames);
    let early = load(&src, &ask(102));
    assert_eq!(early.rows.iter().map(|row| (row.request, row.status, row.requested_hour, row.started_hour)).collect::<Vec<_>>(), [(true, MarketContractStatus::Requested, Some(100), None)], "posting a request starts no contract");
    let later = load(&src, &ask(106));
    let mut seen: Vec<_> = later.rows.iter().map(|row| (row.request, row.status, row.hour, row.ended_hour, row.paid_raw.clone())).collect();
    seen.sort_by_key(|row| row.0);
    assert_eq!(seen, [(false, MarketContractStatus::Active, 103, None, Some("7000".into())), (true, MarketContractStatus::RequestEnded, 100, Some(103), None)], "the removal at acceptance ends the request, not the contract");
}

#[test]
fn a_contract_wide_refusal_and_an_unread_stretch_withhold_the_measures_with_their_reasons() {
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            *frame.tables[TableId::MarketEvents.index()] = if (105..=106).contains(&hour) {
                TableView::Unavailable { reason: "The market was not read".into() }
            } else {
                let events = match hour {
                    100 => vec![event(1, hour, 11, MarketEventRow::default())],
                    102 => vec![event(2, hour, 34, MarketEventRow { mode: 20, ordinal: 4, ..Default::default() }), event(3, hour, 21, MarketEventRow { amount_raw: 5_000, ..Default::default() })],
                    _ => Vec::new(),
                };
                TableView::Complete(TableRows::MarketEvents(events))
            };
            frame
        })
        .collect();
    let src = opened("mkt-refusal", frames);
    let before_gap = load(&src, &ask(104));
    let row = &before_gap.rows[0];
    assert_eq!(row.paid_raw.as_deref(), Some("5000"));
    assert!(row.items.is_empty());
    assert_eq!(row.delivery_missing.as_deref(), Some("Delivery amount unavailable: purchase manifest not acquired before removal"), "a refusal naming no design stands for the whole contract, never \"no deliveries\"");
    let after_gap = load(&src, &ask(110));
    let row = &after_gap.rows[0];
    assert_eq!(row.paid_raw, None, "payments over an unread stretch are not a total");
    assert!(row.paid_missing.as_deref().is_some_and(|reason| reason.contains("not recorded from 105 to 106")), "{:?}", row.paid_missing);
    assert_eq!(after_gap.missing.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(105, 106)]);
}
