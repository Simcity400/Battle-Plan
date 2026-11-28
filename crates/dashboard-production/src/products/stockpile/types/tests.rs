use super::{AI_NEED, Daily, Held, Inputs, Logistic, Need, StockFigure, rows, valid_until};
use super::super::{Domain, StockType, history::Replay};
use dashboard_base::span::HourSpan;
use std::collections::BTreeMap;

fn kind(key: &str, need_note: Option<&str>) -> StockType {
    StockType { key: key.into(), name: Some(key.to_uppercase()), domain: Domain::Land, need_note: need_note.map(str::to_owned) }
}

fn held(id: u64, archetype: Option<&str>, stock: Option<Option<i64>>, daily: Daily) -> Held {
    Held { id, archetype: archetype.map(str::to_owned), token: format!("token_{id}"), name: Some(format!("Design {id}")), origin: Some("GER".into()), stock, daily }
}

fn logistic(balance: i64, stock: i64, available: Option<i64>) -> Vec<Logistic> {
    vec![Logistic { balance, stock, available }]
}

fn raw(figure: &StockFigure) -> Option<&str> {
    figure.raw.as_deref()
}

#[test]
fn stockpile_type_list_distinguishes_empty_from_unread_stock_and_keeps_witnesses() {
    use dashboard_base::test_fixtures::{replay, slot_hour, stockpile_frame};
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::TableView;
    let hour = slot_hour(TableId::Stockpiles) + 24;
    let ask = |stock: &[(u64, i64)], lines: Option<&[(u64, i64, i64, u8)]>, logistics| {
        let mut baseline = stockpile_frame(hour - 1, true, &[], Some(&[]));
        *baseline.tables[TableId::LogisticsEquipment.index()] = TableView::Complete(TableRows::LogisticsEquipment(vec![]));
        let mut frame = stockpile_frame(hour, true, stock, lines);
        *frame.tables[TableId::LogisticsEquipment.index()] = logistics;
        let replay = replay(vec![baseline, frame]).into_timelines(&Default::default()).unwrap();
        let source = super::Source::from_replay(dashboard_base::SourceKey { source_id: "stockpile-types".into(), revision: "1".into() }, "synthetic.battleplan".into(), std::sync::Arc::new(replay), &false.into()).unwrap();
        super::load(&source, &super::StockpileTypesParams { branch_id: "1".into(), country: super::Country::Slot(7), hour }, &super::LoadContext { content: None }, &false.into(), None).unwrap()
    };
    let known = || TableView::Complete(TableRows::LogisticsEquipment(vec![]));
    let empty = ask(&[], Some(&[]), known());
    assert!(empty.rows.is_empty());
    assert_eq!(serde_json::to_value(&empty).unwrap().get("rowsMissing"), Some(&serde_json::Value::Null));
    let unread = ask(&[], None, known());
    assert!(unread.rows.is_empty());
    assert!(serde_json::to_value(&unread).unwrap()["rowsMissing"].as_str().is_some_and(|reason| reason.contains("fixture")));
    let partial = ask(&[(20, 97)], None, known());
    assert_eq!(partial.rows[0].variants[0].stored.raw.as_deref(), Some("97"));
    assert!(serde_json::to_value(&partial).unwrap()["rowsMissing"].is_string());
    let unread = ask(&[(20, 97)], Some(&[]), TableView::Unavailable { reason: "fixture logistics failed".into() });
    assert_eq!(unread.rows.len(), 1);
    assert!(serde_json::to_value(&unread).unwrap()["rowsMissing"].as_str().is_some_and(|reason| reason.contains("fixture logistics failed")));
    let logistics = replay_data::rows::LogisticsEquipmentRow { country: 9001, archetype: 13, token: 1, balance_raw: -37, stock_raw: 0, available_raw: replay_data::rows::Recorded::Value(None) };
    let partial = ask(&[], Some(&[]), TableView::Truncated { rows: TableRows::LogisticsEquipment(vec![logistics]), missing: 2 });
    assert_eq!(partial.rows.len(), 1, "a partially read logistics table still identifies equipment in need");
    assert!(partial.rows[0].balance.missing.as_deref().is_some_and(|reason| reason.contains("2 rows")));
}

#[test]
fn followed_stockpile_bars_keep_unchanged_naming_rows_between_dated_replies() {
    use dashboard_base::{SourceKey, test_fixtures::{played, replay, slot_hour, stockpile_frame}};
    use replay_data::rows::TableId;
    use std::sync::Arc;

    let first = slot_hour(TableId::Stockpiles) + 24;
    let replay = replay((first - 1..=first + 3)
        .map(|hour| stockpile_frame(hour, true, &[(20, 100_000)], Some(&[])))
        .collect()).into_timelines(&Default::default()).unwrap();
    let cancelled = false.into();
    let source = Arc::new(super::Source::from_replay(SourceKey { source_id: "stockpile-bars-warm-names".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap());
    let context = super::LoadContext { content: None };
    let panel = super::panel(&source, &context, &serde_json::json!({ "panel": "equipment-stockpile-types", "branchId": "1", "country": 7 })).unwrap();
    let steps = played(source.replay_ref(), panel.exact().unwrap(), first, first + 3);
    let mut warm = 0;
    for step in steps {
        let params = super::StockpileTypesParams { branch_id: "1".into(), country: super::Country::Slot(7), hour: step.hour };
        let exact = <super::EquipmentStockpileTypes as super::Product>::load(&source, &params, &context, &cancelled).unwrap();
        assert!(!exact.rows.is_empty(), "the requested stock is present");
        assert_eq!(step.value.as_ref(), &Ok(serde_json::to_value(exact).unwrap()));
        if step.hour > first {
            assert_eq!(step.rows, 0, "the numeric and naming inputs were held");
            warm += 1;
        }
    }
    assert!(warm > 0);
}

#[test]
fn each_type_answers_stored_produced_need_and_balance_with_shortages_first_worst_first() {
    let types = [kind("rifles", None), kind("trucks", None), kind("tanks", None), kind("guns", None)];
    let variants = [
        held(1, Some("rifles"), Some(Some(500)), Daily::Units(20)),
        held(2, Some("rifles"), Some(Some(100)), Daily::Idle),
        held(3, Some("trucks"), Some(Some(40)), Daily::Units(3)),
        held(4, Some("tanks"), Some(Some(12)), Daily::Units(1)),
        held(5, Some("guns"), Some(Some(7)), Daily::Idle),
    ];
    let mut by_key = BTreeMap::new();
    by_key.insert("rifles".to_owned(), logistic(10, 600, Some(600)));
    by_key.insert("trucks".to_owned(), logistic(-4, 40, Some(40)));
    by_key.insert("tanks".to_owned(), logistic(-1, 12, Some(12)));
    let need = Need::Player(by_key, None);
    let answered = rows(&Inputs { types: &types, variants: &variants, need: &need, stock_missing: None, lines_missing: None });
    assert_eq!(answered.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), ["trucks", "tanks", "rifles", "guns"], "trucks are short by 4 of 44, tanks by 1 of 13, then the surplus, then a type with no logistics row");
    let rifles = &answered[2];
    assert_eq!((raw(&rifles.stored), raw(&rifles.produced), raw(&rifles.need), raw(&rifles.balance)), (Some("600"), Some("20"), Some("590"), Some("10")));
    assert_eq!(rifles.variants.iter().map(|variant| variant.id.as_str()).collect::<Vec<_>>(), ["1", "2"], "the most stored design first");
    assert_eq!(rifles.variants[0].origin.as_deref(), Some("GER"));
    assert!(answered[0].share.is_some_and(|share| (share + 4.0 / 44.0).abs() < 1e-9));
    assert_eq!(answered[3].balance.missing.as_deref(), Some(super::NO_LOGISTICS), "a type the logistics does not list has its own reason, not zero");
}

#[test]
fn an_ai_country_names_why_need_and_balance_are_missing_and_keeps_database_order() {
    let types = [kind("rifles", None), kind("trucks", None)];
    let variants = [held(1, Some("trucks"), Some(Some(40)), Daily::Units(3)), held(2, Some("rifles"), Some(Some(9)), Daily::Idle)];
    let answered = rows(&Inputs { types: &types, variants: &variants, need: &Need::Ai, stock_missing: None, lines_missing: None });
    assert_eq!(answered.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), ["rifles", "trucks"]);
    for row in &answered {
        assert_eq!((row.need.missing.as_deref(), row.balance.missing.as_deref(), row.share), (Some(AI_NEED), Some(AI_NEED), None));
    }
    assert_eq!(raw(&answered[1].produced), Some("3"));
}

#[test]
fn missing_inputs_are_reasons_never_zero() {
    let types = [kind("rifles", None), kind("convoys", Some("Convoys need the available operand."))];
    let variants = [
        held(1, Some("rifles"), Some(None), Daily::Missing("A line making it has no positive cost, so its output a day is unknown.".into())),
        held(2, Some("convoys"), Some(Some(30)), Daily::Idle),
        held(3, None, Some(Some(5)), Daily::Idle),
    ];
    let mut by_key = BTreeMap::new();
    by_key.insert("convoys".to_owned(), logistic(-2, 30, None));
    let need = Need::Player(by_key, None);
    let answered = rows(&Inputs { types: &types, variants: &variants, need: &need, stock_missing: None, lines_missing: Some("Production lines are not known at this hour.") });
    let rifles = answered.iter().find(|row| row.key == "rifles").unwrap();
    assert_eq!(rifles.stored.missing.as_deref(), Some(super::UNTYPED_HOLDS), "an untyped design holding stock leaves every type's total unproven");
    assert_eq!(rifles.produced.missing.as_deref(), Some(super::UNTYPED_HOLDS));
    let convoys = answered.iter().find(|row| row.key == "convoys").unwrap();
    assert_eq!((raw(&convoys.balance), convoys.need.missing.as_deref()), (Some("-2"), Some("Convoys need the available operand.")), "the type's own need note");
    let untyped = answered.iter().find(|row| row.key.starts_with(super::super::UNRESOLVED_MARK)).unwrap();
    assert_eq!(untyped.stored.missing.as_deref(), Some(super::UNRESOLVED_TYPE));
    assert_eq!(untyped.variants[0].produced.missing.as_deref(), Some("Production lines are not known at this hour."));

    let need = Need::Player(BTreeMap::from([("rifles".into(), logistic(-17, 0, None))]), Some("Logistics read in part".into()));
    let answered = rows(&Inputs { types: &types, variants: &[], need: &need, stock_missing: Some("Stock unread"), lines_missing: Some("Lines unread") });
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0].stored.missing.as_deref(), Some("Stock unread"));
    assert_eq!(answered[0].produced.missing.as_deref(), Some("Lines unread"));
}

#[test]
fn an_answer_stands_until_the_next_read_or_coverage_edge() {
    let span = |first_hour, last_hour| HourSpan { first_hour, last_hour };
    let replay = Replay { reads: vec![10, 34, 58], line_reads: vec![12, 36], logistics_reads: vec![20, 44], stock: vec![span(10, 80)], lines: vec![span(12, 59)], ..Replay::default() };
    assert_eq!(valid_until(&replay, 21, 200, 300), 33, "the stock is read again at 34");
    assert_eq!(valid_until(&replay, 50, 200, 300), 57);
    assert_eq!(valid_until(&replay, 59, 200, 300), 59, "the lines' coverage ends after 59");
    assert_eq!(valid_until(&replay, 70, 75, 300), 75, "the holder's run ends at 75");
    assert_eq!(valid_until(&replay, 90, 200, 95), 95, "the branch ends at 95");
}
