use super::*;
use crate::combat::binding::NoIcons;
use dashboard::modifier_text::{
    BOOLEAN, NEUTRAL, NO_PLUS, PERCENT, PERCENT_TIMES_HUNDRED, POSITIVE_IS_GOOD,
};

fn loc(entries: &[(&str, &str)]) -> Localisation {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn row(flags: u32, precision: i32) -> ModifierDefinitionRow {
    ModifierDefinitionRow {
        name_key: b"MODIFIER_X".to_vec(),
        flags,
        precision,
        category_mask: 1,
        ..Default::default()
    }
}

#[test]
fn value_text_follows_percent_sign_colour_and_postfix_flags() {
    let loc = loc(&[("MOD_DAYS", "days"), ("YES", "Yes"), ("NO", "No")]);
    // Bit 0: the value is a fraction shown as a percentage.
    let percent = row(PERCENT_TIMES_HUNDRED | POSITIVE_IS_GOOD, 1);
    assert_eq!(
        value_text(&percent, 5_000, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}G+5.0%\u{11}!"
    );
    assert_eq!(
        value_text(&percent, -5_000, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}R-5.0%\u{11}!"
    );
    // Without bit 1 a positive value is bad.
    let bad = row(PERCENT_TIMES_HUNDRED, 1);
    assert_eq!(
        value_text(&bad, 5_000, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}R+5.0%\u{11}!"
    );
    let neutral = row(NEUTRAL | NO_PLUS | PERCENT, 0);
    assert_eq!(
        value_text(&neutral, 250_000, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}Y2%\u{11}!"
    );
    let mut days = row(POSITIVE_IS_GOOD, 2);
    days.postfix_key = b"MOD_DAYS".to_vec();
    assert_eq!(
        value_text(&days, 0, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}Y0.00\u{11}! days"
    );
    let boolean = row(BOOLEAN | POSITIVE_IS_GOOD, 0);
    assert_eq!(
        value_text(&boolean, 100_000, 2, false, false, &NoIcons, &loc).unwrap(),
        "\u{11}GYes\u{11}!"
    );
}

#[test]
fn description_orders_by_index_and_skips_hidden_zero_and_uncategorised_entries() {
    let loc = loc(&[
        ("MODIFIER_A", "A"),
        ("MODIFIER_B", "B $ARG$"),
        ("custom_tt", "Custom"),
    ]);
    let mut rows = vec![ModifierDefinitionRow::default(); 5];
    for (index, row) in rows.iter_mut().enumerate() {
        row.index = index as u32;
        row.category_mask = 1;
        row.flags = PERCENT_TIMES_HUNDRED | POSITIVE_IS_GOOD;
        row.precision = 0;
        row.name_key = b"MODIFIER_A".to_vec();
    }
    rows[2].name_key = b"MODIFIER_B".to_vec();
    rows[2].argument = b"Tanks".to_vec();
    rows[3].category_mask = 0;
    rows[4].feature_selector = 4;
    rows[4].eligibility = 1;
    let definitions = Definitions {
        rows: &rows,
        names: std::sync::Arc::new(Ok(BTreeMap::new())),
        features: Some(0),
    };
    let block = Block {
        values: BTreeMap::from([(4, 10_000), (2, -10_000), (1, 10_000), (3, 10_000), (0, 0)]),
        hidden: BTreeSet::from([1]),
        custom_tooltips: keys(&["custom_tt"]),
        added: false,
    };
    assert_eq!(
        block
            .describe(&definitions, &NoIcons, &loc, ONE, "", u32::MAX)
            .unwrap(),
        "B Tanks: \u{11}R-10%\u{11}!\nCustom"
    );
    // A clamped definition cannot show a negative value.
    let mut clamped = rows.clone();
    clamped[2].flags |= CLAMP;
    let definitions = Definitions {
        rows: &clamped,
        names: std::sync::Arc::new(Ok(BTreeMap::new())),
        features: Some(0),
    };
    assert_eq!(
        block
            .describe(&definitions, &NoIcons, &loc, ONE, "", u32::MAX)
            .unwrap(),
        "Custom"
    );
    // An unknown DLC mask refuses a gated entry rather than guessing.
    let definitions = Definitions {
        rows: &rows,
        names: std::sync::Arc::new(Ok(BTreeMap::new())),
        features: None,
    };
    assert!(
        block
            .describe(&definitions, &NoIcons, &loc, ONE, "", u32::MAX)
            .is_err()
    );
    // Each line takes the prefix before its name.
    let definitions = Definitions {
        rows: &rows,
        names: std::sync::Arc::new(Ok(BTreeMap::new())),
        features: Some(0),
    };
    let prefixed = Block {
        values: BTreeMap::from([(0, 10_000), (1, 10_000)]),
        ..Block::default()
    };
    assert_eq!(
        prefixed
            .describe(&definitions, &NoIcons, &loc, ONE, "  ", u32::MAX)
            .unwrap(),
        "  A: \u{11}G+10%\u{11}!\n  A: \u{11}G+10%\u{11}!"
    );
}

#[test]
fn weather_icon_requires_a_nonzero_effective_category_one_entry() {
    let mut rows = vec![row(0, 0); 3];
    for (index, row) in rows.iter_mut().enumerate() {
        row.index = index as u32;
    }
    rows[0].category_mask = 2;
    rows[1].flags = CLAMP;
    rows[2].feature_selector = 4;
    rows[2].eligibility = 1;
    let definitions = Definitions::from_parts(&rows, &[], Some(0));
    let block = Block {
        values: BTreeMap::from([(0, 10_000), (1, -10_000), (2, 10_000)]),
        hidden: BTreeSet::from([2]),
        ..Block::default()
    };
    assert!(!block.has_category_one(&definitions).unwrap());
    let definitions = Definitions::from_parts(&rows, &[], Some(4));
    assert!(block.has_category_one(&definitions).unwrap());
    let mut visible = block;
    visible.values.insert(1, 10_000);
    let definitions = Definitions::from_parts(&rows, &[], None);
    assert!(visible.has_category_one(&definitions).unwrap());
}

/// One CModifier entry `key = value` as the file's reader returns it.
fn entry(key: &str, value: &str) -> installed_content::parser::Item {
    installed_content::parser::Item {
        key: Some(key.as_bytes().to_vec()),
        key_quoted: false,
        operator: installed_content::parser::Operator::Equals,
        value: installed_content::parser::Value::Scalar(value.as_bytes().to_vec()),
        kind: installed_content::parser::Kind::Bare,
    }
}

/// `custom_modifier_tooltip` keys read in order by the shared CModifier
/// reader (1406031A0), in its table's slot order.
fn keys(keys: &[&str]) -> TooltipTable {
    let mut modifiers = installed_content::modifiers::Modifiers::default();
    for key in keys {
        modifiers.read(&entry("custom_modifier_tooltip", key));
    }
    modifiers.tooltip_table().clone()
}

fn listed(table: &TooltipTable) -> Vec<&str> {
    table.keys().collect()
}

/// Only registered rows are recorded, so the table can skip indices (the
/// native gap at 0x29A). A frame recording indices 0, 1 and 3 is
/// admitted, its name index points at index 3, and index 3 formats.
#[test]
fn sparse_definition_tables_are_admitted_and_format_by_index() {
    use replay::frame::{SharedView, TableView};
    use replay::rows::{LexicalTokenRow, TableId, TableRows};
    let mut frame = frame(0, 0);
    frame.tables[TableId::ModifierDefinitions.index()] =
        SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(
            [0, 1, 3]
                .map(|index| ModifierDefinitionRow {
                    index,
                    token: index + 10,
                    name_key: format!("MODIFIER_{index}").into_bytes(),
                    flags: PERCENT,
                    category_mask: 1,
                    precision: 0,
                    ..Default::default()
                })
                .to_vec(),
        )));
    frame.tables[TableId::LexicalTokens.index()] =
        SharedView::from(TableView::Complete(TableRows::LexicalTokens(
            [10, 11, 13]
                .map(|token| LexicalTokenRow {
                    token,
                    literal: format!("key_{}", token - 10).into_bytes(),
                })
                .to_vec(),
        )));
    let definitions = Definitions::read(&frame, &name_index_cache())
        .unwrap()
        .unwrap();
    assert_eq!(definitions.index("key_3"), Ok(Some(3)));
    // The lexer folds a bare token before looking it up (1424A7A90), so
    // an authored `KEY_3` is the recorded `key_3`.
    assert_eq!(definitions.index("KEY_3"), Ok(Some(3)));
    let authored = installed_content::modifiers::values(&[entry("Key_3", "0.1")]);
    let (built, dropped) = Block::build(&definitions, &authored).unwrap();
    assert!(dropped.is_empty());
    assert_eq!(built.values, BTreeMap::from([(3, 10_000)]));
    let loc = loc(&[("MODIFIER_3", "Three")]);
    let block = Block {
        values: BTreeMap::from([(3, 10_000)]),
        ..Block::default()
    };
    let text = block
        .describe(&definitions, &NoIcons, &loc, ONE, "", u32::MAX)
        .unwrap();
    assert!(text.starts_with("Three"), "{text}");
}

#[test]
fn custom_tooltip_keys_follow_the_robin_hood_slot_order() {
    // FNV-1a & 7: k5 -> 5, k6 and k15 -> 4. The first insert grows the
    // shared table to eight buckets; k15 then displaces k5 to slot 6.
    let set = keys(&["k5", "k6", "k15", "k5"]);
    assert_eq!(listed(&set), ["k6", "k15", "k5"]);
    // A copy rebuilds into 2^3 buckets from the source's slot order.
    assert_eq!(listed(&set.copied()), ["k6", "k15", "k5"]);
    // Growth past the 0.9 load of mask 7 keeps every key once.
    let names: Vec<String> = (0..20).map(|index| format!("k{index}")).collect();
    let many = keys(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let mut sorted = listed(&many);
    sorted.sort();
    let mut expected: Vec<&str> = names.iter().map(String::as_str).collect();
    expected.sort();
    assert_eq!(sorted, expected);
    assert_eq!(many.copied().keys().count(), 20);
    let described = Block {
        custom_tooltips: keys(&["b", "a"]),
        ..Block::default()
    };
    let loc = loc(&[("a", "A"), ("b", "B")]);
    let rows: [ModifierDefinitionRow; 0] = [];
    let definitions = Definitions {
        rows: &rows,
        names: std::sync::Arc::new(Ok(BTreeMap::new())),
        features: None,
    };
    // "a" hashes to bucket 4 and "b" to 5: slot order, not read order.
    assert_eq!(
        described
            .describe(&definitions, &NoIcons, &loc, ONE, "", u32::MAX)
            .unwrap(),
        "A\nB"
    );
}

/// The block keeps the reader's source order (1406042A0): a modifier
/// stored visibly, then in `hidden_modifier`, then visibly again takes
/// the last value and stays hidden; visible entries are not applied
/// before hidden ones.
#[test]
fn a_block_keeps_the_readers_last_value_in_source_order() {
    let rows = [ModifierDefinitionRow {
        index: 0,
        token: 1,
        name_key: b"A".to_vec(),
        ..Default::default()
    }];
    let definitions = Definitions::from_parts(&rows, &[(b"a", 0)], Some(0));
    let hidden = installed_content::parser::Item {
        value: installed_content::parser::Value::Block(vec![entry("A", "0.2")]),
        ..entry("hidden_modifier", "")
    };
    let authored = installed_content::modifiers::values(&[entry("a", "0.1"), hidden, entry("a", "0.3")]);
    let (block, dropped) = Block::build(&definitions, &authored).unwrap();
    assert!(dropped.is_empty());
    assert_eq!(block.values, BTreeMap::from([(0, 30_000)]));
    assert_eq!(block.hidden, BTreeSet::from([0]));
}

/// A frame with `definitions` modifier rows and `tokens` dictionary rows.
fn frame(definitions: usize, tokens: usize) -> DecodedFrame {
    use replay::{
        frame::{Metrics, SharedView, TableView},
        rows::{LexicalTokenRow, TableId, TableRows},
    };
    let mut tables: [SharedView; replay::frame::TABLE_COUNT] = std::array::from_fn(|_| {
        SharedView::from(TableView::Unavailable {
            reason: "not recorded".into(),
        })
    });
    tables[TableId::ModifierDefinitions.index()] =
        SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(
            (0..definitions)
                .map(|index| ModifierDefinitionRow {
                    index: index as u32,
                    token: index as u32 + 1,
                    ..Default::default()
                })
                .collect(),
        )));
    tables[TableId::LexicalTokens.index()] =
        SharedView::from(TableView::Complete(TableRows::LexicalTokens(
            (0..tokens as u32)
                .map(|token| LexicalTokenRow {
                    token,
                    literal: format!("modifier_name_{token}").into_bytes(),
                })
                .collect(),
        )));
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables,
        metrics: Metrics::default(),
    }
}

/// Rows sharing a token keep the last registration, the highest index
/// (14054A990 through 140548A80); tokens whose names differ only in case,
/// which the game never holds, are refused by name.
#[test]
fn rows_sharing_a_token_keep_the_last_registration() {
    use replay::{
        frame::{SharedView, TableView},
        rows::{LexicalTokenRow, TableId, TableRows},
    };
    let mut frame = frame(0, 0);
    let row = |index, token| ModifierDefinitionRow {
        index,
        token,
        ..Default::default()
    };
    frame.tables[TableId::ModifierDefinitions.index()] =
        SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(vec![
            row(5, 1),
            row(700, 1),
            row(6, 2),
            row(7, 3),
        ])));
    let token = |token, literal: &str| LexicalTokenRow {
        token,
        literal: literal.as_bytes().to_vec(),
    };
    frame.tables[TableId::LexicalTokens.index()] =
        SharedView::from(TableView::Complete(TableRows::LexicalTokens(vec![
            token(1, "shared"),
            token(2, "Twice"),
            token(3, "twice"),
        ])));
    let names = index_names(&frame).unwrap().unwrap();
    assert_eq!(names[&b"shared"[..]], Some(700));
    assert_eq!(names[&b"twice"[..]], None);
}

#[test]
fn the_name_index_is_built_once_per_static_table_pair() {
    let first = frame(2_000, 70_000);
    let mut next = frame(1, 1);
    next.hour = 2;
    // The next hour shares the static tables, as the decoder does.
    for table in NAME_INDEX_TABLES {
        next.tables[table.index()] = first.tables[table.index()].clone();
    }
    let cache = name_index_cache();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        index_names(&first).unwrap().unwrap();
    }
    let uncached = started.elapsed() / 10;
    let a = Definitions::read(&first, &cache).unwrap().unwrap();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        Definitions::read(&next, &cache).unwrap().unwrap();
    }
    let cached = started.elapsed() / 10;
    let b = Definitions::read(&next, &cache).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&a.names, &b.names));
    assert_eq!(a.index("modifier_name_7"), Ok(Some(6)));
    eprintln!("name index per bind: {uncached:?} rebuilt, {cached:?} cached");
}
