use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records, source};
use replay_data::rows::{LexicalTokenRow, MarketContractTermsRow, MarketEquipment, MarketPool, TableRows, VariantDefinitionRow};
use replay_frame::frame::{DecodedFrame, TableView};

fn opened(id: &str, frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn ask(hour: i32) -> MarketLedgerTailParams {
    MarketLedgerTailParams { branch_id: "1".into(), hour: Some(hour), from: None, to: None, countries: None, group: None, search: None, offset: None, limit: 64, statuses: None, expanded: None, full: None }
}

fn event(id: u64, hour: i32, kind: u8, more: MarketEventRow) -> MarketEventRow {
    MarketEventRow { id, occurrence: id, hour, country: 9001, receiver: 44, kind, agreement: 0x12, ..more }
}

fn delivered(id: u64, hour: i32, variant: u64, amount_raw: i64, total_raw: i64) -> MarketEventRow {
    event(id, hour, 20, MarketEventRow { variant, amount_raw, total_raw, flags: 0x80, ..Default::default() })
}

fn stepped_matches_cold(src: &Source, asked: serde_json::Value, hours: &[i32]) {
    let panel = dated_panel(src, &LoadContext { content: None }, &asked).unwrap();
    let exact = panel.exact().unwrap();
    for (step, &hour) in hours.iter().enumerate() {
        let stepped = (exact.advance)(hour, step == 0, &AtomicBool::new(false), None);
        assert_eq!(*stepped.value, (exact.cold)(hour, &AtomicBool::new(false)), "stepped vs cold at {hour}");
    }
}

fn load(src: &Source, params: &MarketLedgerTailParams) -> MarketLedgerTailReply {
    MarketLedgerTail::load(src, params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
}

fn contracts(reply: &MarketLedgerTailReply) -> Vec<&MarketContractRow> {
    reply.rows.iter().filter_map(|line| if let MarketLedgerLine::Contract(row) = line { Some(row) } else { None }).collect()
}

fn events(reply: &MarketLedgerTailReply) -> Vec<&MarketContractEvent> {
    reply.rows.iter().filter_map(|line| if let MarketLedgerLine::Event(event) = line { Some(event) } else { None }).collect()
}

fn frames(hours: std::ops::RangeInclusive<i32>, events: impl Fn(i32) -> Vec<MarketEventRow>) -> Vec<DecodedFrame> {
    hours
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events(hour)));
            frame
        })
        .collect()
}

#[test]
fn a_contract_is_one_row_with_its_measures_at_the_hour_and_quotes_labelled_per_variant() {
    let (cat, _folder) = dashboard_base::catalogue::tests::localisation_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 }, NBuildings = { MAX_BUILDING_LEVELS = 13 } }"),
        ("localisation/mod_l_english.yml", "\u{feff}l_english:\n rifle_0:0 \"Default Rifle\"\n"),
    ]);
    let context: names::Context = Ok(names::Content::from_catalogue(Arc::new(cat)));
    let frames: Vec<_> = (100..=112)
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"rifle_0".to_vec() }]));
            let name: &[u8] = if hour <= 103 { b"Copperline Field Kit" } else { b"Copperline Field Kit II" };
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow { id: 6, equipment_token: 1, display_name: name.to_vec(), ..Default::default() }]));
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
                104 => vec![delivered(3, 103, 6, 250_000, 250_000)],
                106 => vec![event(4, hour, 21, MarketEventRow { amount_raw: 48_000, ..Default::default() })],
                108 => vec![delivered(5, hour, 6, 250_000, 500_000), event(6, hour, 12, MarketEventRow { flags: 1, ..Default::default() })],
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
    let contract = contracts(&mid)[0];
    assert_eq!((contract.status, contract.live, contract.started_hour, contract.request), (MarketContractStatus::Active, true, Some(100), false));
    assert_eq!(mid.counts[0].total, "1", "the active chip counts the matching population");
    assert_eq!(contract.paid_raw.as_deref(), Some("48000"));
    assert_eq!(contract.quotes, [MarketQuote { variant: "6".into(), name: Some("Copperline Field Kit".into()), name_missing: None, price_raw: "96000".into() }], "a quote is labelled with its variant, never summed");
    assert_eq!((contract.items[0].delivered_raw.as_deref(), contract.items[0].contracted_raw.as_deref()), (Some("250000"), Some("500000")));
    assert_eq!((mid.rows.len(), contract.events_total.as_str()), (1, "3"), "a collapsed contract counts its events and lists none");
    let expanding = MarketLedgerTailParams { expanded: Some(vec![contract.key.clone()]), ..ask(105) };
    let mut opening = prepare(&src, &query, 1, &LoadContext { content: None }, &expanding, &AtomicBool::new(false)).unwrap();
    opening.context = Arc::clone(&prepared.context);
    opening.namings = Arc::clone(&prepared.namings);
    let open = load_tail_at(&opening, &query, &expanding, 105, &AtomicBool::new(false)).unwrap();
    assert!(matches!(open.rows[0], MarketLedgerLine::Contract(_)), "an opened contract's events follow it");
    let listed = contracts(&open)[0];
    assert_eq!(events(&open).iter().map(|ev| (ev.hour, ev.kind)).collect::<Vec<_>>(), [(103, MarketContractEventKind::Delivery), (102, MarketContractEventKind::Payment), (100, MarketContractEventKind::ContractStarted)]);
    assert_eq!(events(&open)[0].equipment.as_deref(), Some("Copperline Field Kit"), "a delivery read at 104 names its design at its own hour, 103, not at the hour it was read");
    assert_eq!((&listed.items, &listed.quotes, &listed.paid_raw), (&contract.items, &contract.quotes, &contract.paid_raw), "expanding a contract changes none of its row's measures");
    let unterms: Vec<_> = (100..=112)
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"rifle_0".to_vec() }]));
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow { id: 6, equipment_token: 1, display_name: b"Brasswick Rifle Set".to_vec(), ..Default::default() }]));
            let events = match hour {
                100 => vec![event(1, hour, 11, MarketEventRow::default())],
                104 => vec![event(3, hour, 20, MarketEventRow { variant: 6, amount_raw: 250_000, ..Default::default() })],
                _ => Vec::new(),
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let bare = opened("mkt-unterms", unterms);
    let bare_query = dashboard_base::queries::query(&bare, 1).unwrap();
    let mut bare_prepared = prepare(&bare, &bare_query, 1, &LoadContext { content: None }, &ask(105), &AtomicBool::new(false)).unwrap();
    bare_prepared.context = prepared.context.clone();
    bare_prepared.namings = Arc::new(namings(&bare, &bare_query).unwrap());
    let unopened = load_tail_at(&bare_prepared, &bare_query, &ask(105), 105, &AtomicBool::new(false)).unwrap();
    assert_eq!(contracts(&unopened)[0].items[0].name.as_deref(), Some("Brasswick Rifle Set"), "a design the contract only received is named at the contract's start, whatever else is on the page or opened");
    assert_eq!(
        (contracts(&unopened)[0].items[0].delivered_raw.as_deref(), contracts(&unopened)[0].items[0].delivery_unavailable.as_deref()),
        (Some("250000"), None),
        "a delivery with no cumulative total counts its own amount, in full when every hour since the start was read"
    );
    assert_eq!(mid.next_change, Some(106), "the next payment is the next change");
    let done = load_tail_at(&prepared, &query, &ask(110), 110, &AtomicBool::new(false)).unwrap();
    let finished = contracts(&done)[0];
    assert_eq!((finished.status, finished.ended_hour, finished.paid_raw.as_deref()), (MarketContractStatus::Ended, Some(108), Some("96000")));
    assert_eq!(finished.events_total, "6");
    let ended_only = load(&src, &MarketLedgerTailParams { statuses: Some(vec![MarketContractStatus::Active]), ..ask(110) });
    assert_eq!(ended_only.total, "0", "the status filter applies at the hour asked, before counting");
    stepped_matches_cold(&src, serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "limit": 64, "countries": ["GER"] }), &[100, 102, 103, 105, 108, 104, 112]);
}

#[test]
fn a_design_whose_token_the_run_never_spells_keeps_the_refusal_of_its_own_hour() {
    let (cat, _folder) = dashboard_base::catalogue::tests::localisation_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 }, NBuildings = { MAX_BUILDING_LEVELS = 13 } }"),
        ("localisation/mod_l_english.yml", "\u{feff}l_english:\n rifle_0:0 \"Default Rifle\"\n"),
    ]);
    let context: names::Context = Ok(names::Content::from_catalogue(Arc::new(cat)));
    let frames: Vec<_> = (100..=106)
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            *frame.tables[TableId::LexicalTokens.index()] = if hour == 100 {
                TableView::Unavailable { reason: "synthetic lexical refusal".into() }
            } else {
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 2, literal: b"rifle_0".to_vec() }]))
            };
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(vec![
                VariantDefinitionRow { id: 6, equipment_token: 1, display_name: b"Stored Design".to_vec(), ..Default::default() },
                VariantDefinitionRow { id: 7, equipment_token: 2, display_name: b"Later Spelled".to_vec(), ..Default::default() },
            ]));
            if hour == 100 {
                *frame.tables[TableId::MarketContractTerms.index()] = TableView::Complete(TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                    country: 9001,
                    buyer: 44,
                    id: 0x12,
                    kind: 1,
                    equipment: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 500_000 }] }.into(),
                    prices: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 96_000 }, MarketEquipment { variant: 7, amount_raw: 12_000 }] }.into(),
                    ..Default::default()
                }]));
            }
            let events = if hour == 100 { vec![event(1, hour, 11, MarketEventRow::default())] } else { Vec::new() };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let src = opened("mkt-unspelled", frames);
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let mut prepared = prepare(&src, &query, 1, &LoadContext { content: None }, &ask(104), &AtomicBool::new(false)).unwrap();
    prepared.context = Arc::new(context);
    prepared.namings = Arc::new(namings(&src, &query).unwrap());
    let reply = load_tail_at(&prepared, &query, &ask(104), 104, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        contracts(&reply)[0].quotes,
        [
            MarketQuote { variant: "6".into(), name: None, name_missing: Some(UNNAMED.into()), price_raw: "96000".into() },
            MarketQuote { variant: "7".into(), name: Some("Later Spelled".into()), name_missing: None, price_raw: "12000".into() },
        ],
        "the hour's lexical refusal stands for a token the run never spells, and a token the run spells later still names its design in the same reply"
    );
}

#[test]
fn a_removal_ends_only_the_kind_it_records_and_an_isolated_one_makes_no_contract() {
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let events = match hour {
                100 => vec![event(1, hour, 10, MarketEventRow::default())],
                103 => vec![event(2, hour, 11, MarketEventRow::default()), event(3, hour, 12, MarketEventRow::default())],
                104 => vec![event(4, hour, 21, MarketEventRow { amount_raw: 7_000, ..Default::default() })],
                106 => vec![event(5, hour, 12, MarketEventRow::default())],
                107 => vec![event(6, hour, 12, MarketEventRow { flags: 2, ..Default::default() })],
                108 => vec![MarketEventRow { agreement: 0x13, ..event(7, hour, 12, MarketEventRow { flags: 1, ..Default::default() }) }],
                109 => vec![
                    MarketEventRow { agreement: 0x14, ..event(8, hour, 11, MarketEventRow::default()) },
                    MarketEventRow { agreement: 0x14, ..event(9, hour, 12, MarketEventRow { flags: 1, ..Default::default() }) },
                    MarketEventRow { agreement: 0x14, occurrence: 9, ..event(10, hour, 34, MarketEventRow { mode: 20, ordinal: 4, ..Default::default() }) },
                ],
                _ => Vec::new(),
            };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let src = opened("mkt-request", frames);
    let early = load(&src, &ask(102));
    assert_eq!(contracts(&early).iter().map(|row| (row.request, row.status, row.requested_hour, row.started_hour)).collect::<Vec<_>>(), [(true, MarketContractStatus::Requested, Some(100), None)], "posting a request starts no contract");
    let seen = |reply: &MarketLedgerTailReply| {
        let mut seen: Vec<_> = contracts(reply).iter().map(|row| (row.request, row.status, row.hour, row.ended_hour, row.paid_raw.clone())).collect();
        seen.sort_by_key(|row| (row.0, row.2));
        seen
    };
    let untold = |reply: &MarketLedgerTailReply| reply.warnings.iter().filter(|warning| warning.starts_with("Market events not listed")).cloned().collect::<Vec<_>>();
    let accepted = load(&src, &ask(105));
    assert_eq!(seen(&accepted), [(false, MarketContractStatus::Active, 103, None, Some("7000".into())), (true, MarketContractStatus::RequestEnded, 100, Some(103), None)], "the removal at acceptance ends the request, not the contract");
    let isolated = load(&src, &ask(106));
    assert_eq!(
        seen(&isolated),
        [(false, MarketContractStatus::Active, 103, None, Some("7000".into())), (true, MarketContractStatus::RequestEnded, 100, Some(103), None), (true, MarketContractStatus::RequestEnded, 106, Some(106), None)],
        "a request removal with no open request is an ended request of unrecorded start, and leaves the open contract running"
    );
    assert!(untold(&isolated).is_empty());
    assert_eq!(isolated.next_change, Some(107));
    let unsaid = load(&src, &ask(107));
    assert_eq!(contracts(&unsaid).len(), 3, "a removal that does not record its kind ends nothing");
    assert_eq!(untold(&unsaid), ["Market events not listed because they do not record whether a removal ended a request or an accepted contract: 1"]);
    let lone = load(&src, &ask(109));
    let ended = contracts(&lone).into_iter().find(|row| row.hour == 108).unwrap();
    assert_eq!(
        (ended.request, ended.status, ended.exact, ended.paid_raw.as_deref(), ended.paid_missing.as_deref()),
        (false, MarketContractStatus::Ended, false, None, Some(START_UNSEEN)),
        "a contract removal with no open contract is a contract whose start and payments were not recorded"
    );
    let refused = load(&src, &ask(110));
    let terminal: Vec<_> = contracts(&refused).into_iter().filter(|row| row.hour == 109).map(|row| (row.status, row.live, row.events_total.clone(), row.delivery_missing.clone())).collect();
    assert_eq!(
        terminal,
        [(MarketContractStatus::Ended, false, "3".into(), Some("Delivery amount unavailable: purchase manifest not acquired before removal".into()))],
        "a refusal published after its removal, from the removal's own occurrence, belongs to the ended contract and starts no other"
    );
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
                    101 => vec![
                        MarketEventRow { agreement: 0x13, ..event(4, hour, 21, MarketEventRow { amount_raw: 2_000, ..Default::default() }) },
                        MarketEventRow { agreement: 0x13, ..delivered(5, hour, 6, 3_000, 9_000) },
                        MarketEventRow { agreement: 0x13, ..event(8, hour, 20, MarketEventRow { variant: 7, amount_raw: 4_000, ..Default::default() }) },
                    ],
                    102 => vec![event(2, hour, 34, MarketEventRow { mode: 20, ordinal: 4, ..Default::default() }), event(3, hour, 21, MarketEventRow { amount_raw: 5_000, ..Default::default() })],
                    107 => vec![event(6, 104, 21, MarketEventRow { amount_raw: 1_000, ..Default::default() })],
                    109 => vec![event(7, hour, 21, MarketEventRow { amount_raw: 1_000, ..Default::default() }), delivered(9, hour, 6, 900_000, 900_000)],
                    _ => Vec::new(),
                };
                TableView::Complete(TableRows::MarketEvents(events))
            };
            if hour == 100 {
                *frame.tables[TableId::MarketContractTerms.index()] = TableView::Complete(TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                    country: 9001,
                    buyer: 44,
                    id: 0x12,
                    kind: 1,
                    equipment: MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 2_000_000 }] }.into(),
                    ..Default::default()
                }]));
            }
            frame
        })
        .collect();
    let src = opened("mkt-refusal", frames);
    let witnessed = |reply: &MarketLedgerTailReply| contracts(reply).into_iter().find(|row| row.exact).unwrap().clone();
    let before_gap = load(&src, &ask(104));
    let row = witnessed(&before_gap);
    assert_eq!((row.status, row.paid_raw.as_deref(), row.paid_missing.as_deref()), (MarketContractStatus::Active, Some("5000"), None));
    let refusal = "Delivery amount unavailable: purchase manifest not acquired before removal";
    assert_eq!(
        row.items.iter().map(|item| (item.variant.as_str(), item.delivered_raw.as_deref(), item.delivery_unavailable.as_deref())).collect::<Vec<_>>(),
        [("6", None, Some(refusal))]
    );
    assert_eq!(row.delivery_missing.as_deref(), Some(refusal), "a refusal naming no design stands for the whole contract, never \"no deliveries\"");
    let unseen = contracts(&before_gap).into_iter().find(|row| !row.exact).unwrap();
    assert_eq!(
        (unseen.hour, unseen.started_hour, unseen.paid_raw.as_deref(), unseen.paid_missing.as_deref()),
        (101, None, Some("2000"), Some(START_UNSEEN)),
        "the payments of a contract first seen after its start are at least those seen"
    );
    assert_eq!(
        (unseen.items[0].delivered_raw.as_deref(), unseen.items[0].delivery_unavailable.as_deref(), unseen.delivery_missing.as_deref()),
        (Some("9000"), None, Some(START_UNSEEN)),
        "a delivery's certified cumulative total is the design's whole delivery, even when the start was not recorded"
    );
    assert_eq!(
        (unseen.items[1].delivered_raw.as_deref(), unseen.items[1].delivery_unavailable.as_deref()),
        (Some("4000"), Some(START_UNSEEN)),
        "a delivery with no cumulative total, seen after an unrecorded start, is at least its own amount"
    );
    let inside = witnessed(&load(&src, &ask(105)));
    assert_eq!(inside.status, MarketContractStatus::NotRecorded, "an unread stretch with no recorded end leaves the status unknown");
    assert!(inside.paid_missing.as_deref().is_some_and(|reason| reason.contains("not recorded from 105 to 106")), "the reason names the whole unread stretch from its first hour: {:?}", inside.paid_missing);
    let delayed = load(&src, &ask(107));
    assert_eq!(
        (witnessed(&delayed).status, witnessed(&delayed).paid_raw.as_deref()),
        (MarketContractStatus::NotRecorded, Some("6000")),
        "a payment dated before the unread stretch counts from when it was read, yet does not show the contract alive after the stretch"
    );
    assert_eq!(delayed.counts[0].total, "0", "a contract of unknown status is not counted active");
    let after_gap = load(&src, &ask(110));
    let row = witnessed(&after_gap);
    assert_eq!((row.status, row.paid_raw.as_deref()), (MarketContractStatus::Active, Some("7000")), "a payment dated after the unread stretch shows the contract alive, and the payments stay a lower bound");
    assert_eq!(
        (row.items[0].delivered_raw.as_deref(), row.items[0].contracted_raw.as_deref(), row.items[0].delivery_unavailable.as_deref(), row.delivery_missing.as_deref()),
        (Some("900000"), Some("2000000"), None, None),
        "a certified total after the stretch and the refusal covers the only design of the terms, so the contract's deliveries are known"
    );
    assert!(row.paid_missing.as_deref().is_some_and(|reason| reason.contains("not recorded from 105 to 106")), "{:?}", row.paid_missing);
    let unseen = contracts(&after_gap).into_iter().find(|row| !row.exact).unwrap();
    assert_eq!(unseen.items[0].delivered_raw.as_deref(), Some("9000"));
    assert!(unseen.items[0].delivery_unavailable.as_deref().is_some_and(|reason| reason.contains("not recorded from 105 to 106")), "an unread stretch after a certified total leaves it a lower bound");
    assert_eq!(after_gap.counts[0].total, "1");
    assert_eq!(after_gap.missing.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(105, 106)]);
    stepped_matches_cold(&src, serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "limit": 64 }), &(100..=110).collect::<Vec<_>>());
}

#[test]
fn an_event_is_dated_by_its_own_hour_and_listed_from_the_hour_it_was_read() {
    let src = opened(
        "mkt-late",
        frames(100..=110, |hour| match hour {
            102 => vec![event(1, 101, 11, MarketEventRow::default())],
            105 => vec![event(2, 104, 21, MarketEventRow { amount_raw: 5_000, ..Default::default() }), delivered(4, 103, 6, 2_000, 2_000)],
            107 => vec![event(3, 106, 12, MarketEventRow { flags: 1, ..Default::default() })],
            _ => Vec::new(),
        }),
    );
    assert_eq!(load(&src, &ask(101)).total, "0", "a contract is not listed before it was read");
    let started = load(&src, &ask(102));
    let row = contracts(&started)[0];
    assert_eq!((row.hour, row.observed, row.started_hour, row.paid_raw.as_deref()), (101, 102, Some(101), None), "an acceptance is dated by its own hour");
    assert_eq!(started.next_change, Some(105));
    let paid = load(&src, &MarketLedgerTailParams { expanded: Some(vec![row.key.clone()]), ..ask(106) });
    let contract = contracts(&paid)[0];
    assert_eq!((contract.status, contract.ended_hour, contract.paid_raw.as_deref()), (MarketContractStatus::Active, None, Some("5000")), "a removal is not applied before it was read");
    assert_eq!(
        events(&paid).iter().map(|ev| (ev.hour, ev.kind)).collect::<Vec<_>>(),
        [(104, MarketContractEventKind::Payment), (103, MarketContractEventKind::Delivery), (101, MarketContractEventKind::ContractStarted)],
        "events are dated by their own hour and listed newest first, whatever order one read published them in"
    );
    let ended = load(&src, &ask(107));
    assert_eq!((contracts(&ended)[0].status, contracts(&ended)[0].ended_hour), (MarketContractStatus::Ended, Some(106)));
}

#[test]
fn an_opened_contract_pages_its_events_after_it_and_lists_the_newest_until_asked_for_all() {
    let src = opened(
        "mkt-paged",
        frames(100..=142, |hour| match hour {
            100 => vec![event(1, hour, 11, MarketEventRow::default())],
            101..=140 => vec![event(hour as u64, hour, 21, MarketEventRow { amount_raw: 1_000, ..Default::default() })],
            141 => vec![event(500, 101, 21, MarketEventRow { amount_raw: 1_000, ..Default::default() })],
            _ => Vec::new(),
        }),
    );
    let key = contracts(&load(&src, &ask(141)))[0].key.clone();
    let opened = MarketLedgerTailParams { expanded: Some(vec![key.clone()]), ..ask(141) };
    let capped = load(&src, &opened);
    assert_eq!(capped.total, "34", "a contract, its newest 32 events and the line naming the rest");
    assert_eq!(
        (contracts(&capped).len(), events(&capped).first().map(|ev| ev.hour), events(&capped).last().map(|ev| ev.hour)),
        (1, Some(140), Some(109)),
        "an event read late but dated early never displaces a newer one"
    );
    assert_eq!(capped.rows.last(), Some(&MarketLedgerLine::More(MarketEventsMore { key: format!("more:{key}"), hour: 100, contract: key.clone(), total: "42".into(), hidden: "10".into() })));
    assert_eq!((capped.counts[0].total.as_str(), capped.live.as_str()), ("1", "1"), "event lines are not counted as contracts, nor as live ones");
    let all = MarketLedgerTailParams { full: Some(vec![key.clone()]), ..opened.clone() };
    let every = load(&src, &all);
    assert_eq!(every.total, "44", "asking for every event lists them all");
    assert!(matches!(every.rows.last(), Some(MarketLedgerLine::More(more)) if more.hidden == "0"));
    let page = load(&src, &MarketLedgerTailParams { offset: Some("35".into()), limit: 4, ..all });
    assert_eq!(events(&page).iter().map(|ev| ev.hour).collect::<Vec<_>>(), [106, 105, 104, 103], "an opened contract's events page like any other line");
    let bounded = load(&src, &MarketLedgerTailParams { to: Some(135), ..opened.clone() });
    assert_eq!(
        (events(&bounded).first().map(|ev| ev.hour), events(&bounded).last().map(|ev| ev.hour), bounded.rows.last()),
        (Some(135), Some(104), Some(&MarketLedgerLine::More(MarketEventsMore { key: format!("more:{key}"), hour: 100, contract: key.clone(), total: "37".into(), hidden: "5".into() }))),
        "the asked range bounds each event by its own hour before the newest are listed and the rest counted"
    );
    let early = load(&src, &MarketLedgerTailParams { hour: Some(120), ..opened });
    assert_eq!((early.total.as_str(), early.rows.iter().any(|line| matches!(line, MarketLedgerLine::More(_)))), ("22", false), "the cap counts only the events read by the hour asked");
    stepped_matches_cold(&src, serde_json::json!({ "panel": "market-ledger-tail", "branchId": "1", "limit": 64, "expanded": [key] }), &[120, 131, 132, 133, 140, 141, 136]);
}

#[test]
fn a_party_is_named_at_the_contracts_date_and_a_buyer_first_recorded_later_only_from_then() {
    let relinked: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = ownership_frame(1, hour, 9001);
            if hour > 100
                && let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.iter_mut().filter(|row| row.id == 9001).for_each(|row| row.canonical_slot = 10);
            }
            let events = if hour == 102 { vec![event(1, 100, 11, MarketEventRow::default())] } else { Vec::new() };
            *frame.tables[TableId::MarketEvents.index()] = TableView::Complete(TableRows::MarketEvents(events));
            frame
        })
        .collect();
    let moved = opened("mkt-relinked", relinked);
    let picked = |tag: &str| load(&moved, &MarketLedgerTailParams { countries: Some(vec![Country::Tag(tag.into())]), ..ask(103) }).total;
    let searched = |text: &str| load(&moved, &MarketLedgerTailParams { search: Some(text.into()), ..ask(103) }).total;
    assert_eq!(contracts(&load(&moved, &ask(103)))[0].seller.as_deref(), Some("GER"), "the seller is named as it was at the acceptance, not at the later read");
    assert_eq!(
        (picked("GER"), picked("MOD"), searched("GER"), searched("MOD")),
        ("1".to_owned(), "0".to_owned(), "1".to_owned(), "0".to_owned()),
        "selection and search see the seller of the acceptance's own hour"
    );
    let src = opened(
        "mkt-buyer",
        frames(100..=106, |hour| match hour {
            100 => vec![MarketEventRow { receiver: 0, ..event(1, hour, 11, MarketEventRow::default()) }],
            104 => vec![MarketEventRow { receiver: 66, ..event(2, hour, 21, MarketEventRow { amount_raw: 1_000, ..Default::default() }) }],
            _ => Vec::new(),
        }),
    );
    let chosen = |hour| MarketLedgerTailParams { countries: Some(vec![Country::Tag("MOD".into())]), ..ask(hour) };
    assert_eq!(load(&src, &chosen(103)).total, "0", "a buyer not yet recorded does not admit the contract");
    let known = load(&src, &chosen(104));
    assert_eq!((known.total.as_str(), contracts(&known)[0].buyer.as_deref()), ("1", Some("MOD")));
    assert_eq!(contracts(&load(&src, &ask(103)))[0].buyer, None, "the buyer is missing until it is recorded");
    assert_eq!(load(&src, &MarketLedgerTailParams { search: Some("GER".into()), ..ask(103) }).total, "1", "the seller known at the opening is searchable");
    assert_eq!(load(&src, &MarketLedgerTailParams { search: Some("MOD".into()), ..ask(106) }).total, "0", "a buyer recorded later never matches a search, which cannot tell when it was recorded");
}
