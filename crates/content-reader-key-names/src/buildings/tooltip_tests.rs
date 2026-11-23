use super::*;
#[test]
fn tooltip_operands_follow_authored_lists_and_last_valid_lane() {
    content_reader_script::test_game::enter();
    let data = parse(
        br#"buildings = {
        laboratory = { specialization = { alpha } specialization = { beta alpha }
            detecting_intel_type = army detecting_intel_type = navy }
        ordinary = { } broken = { detecting_intel_type = unsupported show_on_map = 7 }
    }"#,
        &KeyNames::default(),
    )
    .unwrap();
    let unreadable = parse(
        b"buildings = { a = { specialization = { \"\xe9\" } show_on_map = 7 } b = { tags = { \"\xe9\" } } }", &KeyNames::default(),
    )
    .unwrap();
    let lab = &data.types["laboratory"];
    assert_eq!(lab.specializations, ["alpha", "beta", "alpha"]);
    assert_eq!(lab.detecting_intel_type, 2);
    assert_eq!(lab.tooltip_refused, None);
    assert_eq!(data.types["ordinary"].detecting_intel_type, 0);
    let broken = &data.types["broken"];
    assert_eq!(broken.detecting_intel_type, 0);
    assert_eq!(broken.tooltip_refused, None);
    assert_eq!(broken.show_on_map, Some(7));
    let a = &unreadable.types["a"];
    assert!(a.tooltip_refused.as_ref().unwrap().contains("UTF-8"));
    assert_eq!((a.show_on_map, &a.raid_refused), (Some(7), &None));
    assert!(
        unreadable.types["b"]
            .raid_refused
            .as_ref()
            .unwrap()
            .contains("UTF-8")
    );
}
