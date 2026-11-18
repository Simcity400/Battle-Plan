use super::rows_differing;
use replay_data::rows::{CountryRow, TableRows};

#[test]
fn row_proof_keeps_field_differences_and_both_missing_sides() {
    let country = |id, tag: &[u8]| CountryRow {
        id,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let got = TableRows::Countries(vec![
        country(1, b"head"),
        country(3, b"three"),
        country(4, b"four"),
    ]);
    let wanted = TableRows::Countries(vec![
        country(1, b"today"),
        country(2, b"two"),
        country(4, b"four"),
    ]);
    assert_eq!(
        rows_differing(&got, &wanted),
        (
            3,
            Some("key 1: at the head 1=\"head\", today 1=\"today\"".into())
        )
    );
    assert_eq!(rows_differing(&got, &got), (0, None));
    assert_eq!(
        rows_differing(
            &TableRows::Countries(vec![country(3, b"three")]),
            &TableRows::Countries(vec![country(2, b"two")])
        ),
        (2, Some("key 2 only today".into()))
    );
}

#[test]
fn row_proof_refuses_another_table_even_when_both_are_empty() {
    assert_eq!(
        rows_differing(
            &TableRows::Countries(Vec::new()),
            &TableRows::Clock(Vec::new())
        ),
        (1, Some("the two hold rows of different tables".into()))
    );
}
