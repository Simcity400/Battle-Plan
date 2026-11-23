use super::*;

fn decisions(text: &[u8]) -> io::Result<Decisions> {
    parse_decisions(&content_reader_parser::parser::parse(text)?.items)
}

#[test]
fn decision_random_items_preserve_signed_words_order_and_repeats() {
    let saved = decisions(
        b"random_item={decision=mission count=-2147483648 target=-2147483648} random_item={decision=mission count=2147483647 target=-2147483648}",
    ).unwrap();
    assert_eq!(
        saved.random_items,
        vec![
            DecisionRandomItem {
                token: b"mission".to_vec(),
                count: i32::MIN,
                target: i32::MIN
            },
            DecisionRandomItem {
                token: b"mission".to_vec(),
                count: i32::MAX,
                target: i32::MIN
            },
        ]
    );
    assert!(decisions(b"").unwrap().random_items.is_empty());
    for text in [
        b"random_item={decision=mission target=0}".as_slice(),
        b"random_item={decision=mission count=0}",
        b"random_item={count=0 target=0}",
        b"random_item={decision=mission count=2147483648 target=0}",
        b"random_item={decision=mission count=0 count=1 target=0}",
    ] {
        assert!(
            decisions(text).is_err(),
            "{}",
            String::from_utf8_lossy(text)
        );
    }
}
