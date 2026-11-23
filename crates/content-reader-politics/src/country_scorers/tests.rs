use super::*;

#[test]
fn scorer_entries_keep_target_arrays_filters_and_scores() {
    content_reader_script::test_game::enter();
    let mut database = Database::default();
    database.read_file(b"sample={targets={target_array=global.sample targets={AAA BBB} country=CCC target_root_trigger={always=yes} target_trigger={FROM={exists=yes}} targets_dynamic=yes targets_dynamic=no target_non_existing=yes score={base=2 modifier={always=yes add=3}} score={base=4}} targets={targets={AAA}}}", &KeyNames::default());
    let entries = database.get(b"SAMPLE").unwrap().unwrap();
    assert_eq!(entries.len(), 2);
    let entry = &entries[0];
    assert_eq!(entry.targets.arrays, [b"global.sample".to_vec()]);
    assert_eq!(
        entry.targets.targets,
        [b"AAA".to_vec(), b"BBB".to_vec(), b"CCC".to_vec()]
    );
    assert_eq!(entry.targets.root_limit.len(), 1);
    assert_eq!(entry.targets.target_limit.len(), 1);
    assert!(!entry.targets.dynamic);
    assert!(entry.targets.non_existing);
    assert_eq!(entry.score.len(), 3);
    assert!(entry.score.last().unwrap().keyword_is("base"));
    assert!(database.notes.is_empty(), "{:?}", database.notes);
}

#[test]
fn duplicate_scorer_is_skipped_and_unknown_fields_are_named() {
    content_reader_script::test_game::enter();
    let mut database = Database::default();
    database.read_file(
        b"sample={targets={targets={AAA}}} SAMPLE={targets={targets={BBB}}}",
        &KeyNames::default(),
    );
    database.read_file(b"sample={targets={targets={CCC}}}", &KeyNames::default());
    assert_eq!(
        database.get(b"sample").unwrap().unwrap()[0].targets.targets,
        [b"AAA".to_vec()]
    );
    database.read_file(b"bad={targets={unknown_field=yes}}", &KeyNames::default());
    let why = database.get(b"bad").unwrap_err();
    assert!(
        why.contains("unknown_field") && why.contains("country scorer target field"),
        "{why}"
    );
}
