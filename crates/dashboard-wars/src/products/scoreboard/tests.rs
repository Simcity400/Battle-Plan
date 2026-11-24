use super::*;

#[test]
fn every_column_but_the_combined_casualties_reads_a_declared_value_of_a_country_and_casualties_name_their_partial_mark() {
    let columns = registry(|_| None);
    assert_eq!(columns.len(), DEFS.len());
    for column in &columns {
        if column.key == ScoreboardColumnKey::Casualties {
            assert!(!column.available && column.family.is_none() && column.reason.is_some());
            continue;
        }
        assert!(column.available, "{:?}: {:?}", column.key, column.reason);
        let family = column.family.as_ref().unwrap();
        let (_, measure) = declared((family.kind.as_str(), family.measure.as_str())).unwrap();
        assert_eq!(column.decimals, measure.decimals);
    }
    for key in [ScoreboardColumnKey::CasualtiesDealt, ScoreboardColumnKey::CasualtiesTaken] {
        let column = columns.iter().find(|column| column.key == key).unwrap();
        assert_eq!(column.partial, Some(ScoreboardFamily { kind: "casualty-totals".into(), measure: "partial".into() }));
        assert!(column.none_at_zero, "{key:?}: a country with no casualty row has dealt or taken none");
    }
    assert!(columns.iter().filter(|column| !matches!(column.key, ScoreboardColumnKey::CasualtiesDealt | ScoreboardColumnKey::CasualtiesTaken)).all(|column| !column.none_at_zero),
        "a missing row of any other measure stays missing");
    assert!(DEFAULT_COLUMNS.iter().all(|key| columns.iter().any(|column| column.key == *key && column.available)));
    assert!(columns.iter().any(|column| column.key == DEFAULT_SORT && column.available));
}

