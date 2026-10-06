use super::*;

#[test]
fn collection_constants_preserve_native_variants_and_array_order() {
    let constants = parse(&[(
        "synthetic.txt",
        b"states={schema={any_key=yes array=state} group={8 7 0 -1 010 2tail}} \
        tags={schema={any_key=yes array=country} group={BBB AAA MISSING}} \
        scalar={schema={any_key=yes data=state} valid=7 invalid=0}",
    )]);
    assert_eq!(
        constants.collection(&["states", "group"]).unwrap(),
        Some(&CollectionConstant::States(vec![8, 7, 0, -1, 10, 2]))
    );
    assert_eq!(
        constants.collection(&["tags", "group"]).unwrap(),
        Some(&CollectionConstant::Countries(vec![
            b"BBB".to_vec(),
            b"AAA".to_vec(),
            b"MISSING".to_vec()
        ]))
    );
    assert_eq!(
        constants.collection(&["scalar", "valid"]).unwrap(),
        Some(&CollectionConstant::State(7))
    );
    assert_eq!(constants.collection(&["scalar", "invalid"]).unwrap(), None);
    assert!(matches!(
        constants.value(&["states", "group"]),
        Constant::Unset(_)
    ));
}
