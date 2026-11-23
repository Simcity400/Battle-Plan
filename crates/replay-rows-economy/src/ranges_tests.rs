use crate::ConstructionLineOutputRow;
use replay_rows::sliced::ranges::{apply, growth};

fn rows(lines: &[u64]) -> Vec<ConstructionLineOutputRow> {
    lines
        .iter()
        .map(|&line| ConstructionLineOutputRow {
            line,
            ..Default::default()
        })
        .collect()
}

#[test]
fn replacement_reserves_the_largest_intermediate_length() {
    let ranges = [1..3, 10..12];
    assert_eq!(growth(&rows(&[1, 2]), &rows(&[10, 11]), &ranges), (2, 2));
    assert_eq!(growth(&rows(&[1, 2]), &rows(&[2, 10]), &ranges), (2, 1));
    assert_eq!(growth(&rows(&[10, 11]), &rows(&[1, 2]), &ranges), (2, 0));
    let mut old = rows(&[1, 2]);
    let retired = apply(&mut old, rows(&[10, 11]), &ranges).unwrap();
    assert_eq!((old, retired), (rows(&[10, 11]), rows(&[1, 2])));
}
