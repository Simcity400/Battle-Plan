use super::*;
use content_reader_script::key_names::KeyNames;

#[test]
fn infrastructure_identity_is_independent_of_runtime_limit_forms() {
    content_reader_script::test_game::enter();
    for form in ["land_fort=1", "naval_fort=2", "level_cap={state_max=5 exclusive_with=canal}", "level_cap={state_max=5 shares_slots=yes}", "level_cap={state_max=5 group_by=paths}"] {
        let source = format!("buildings={{plain={{}} roads={{infrastructure=yes {form}}}}}");
        let buildings = content_reader_key_names::buildings::parse(source.as_bytes(), &KeyNames::default()).unwrap();
        let building = Content::new(&buildings).building.unwrap();
        assert_eq!(building.name, "roads");
        assert!(building.in_states);
    }
}

#[test]
fn state_status_availability_keeps_provincial_and_coastal_types_black() {
    content_reader_script::test_game::enter();
    for scope in ["level_cap={province_max=3}", "only_costal=yes level_cap={state_max=5}"] {
        let source = format!("buildings={{roads={{infrastructure=yes {scope}}}}}");
        let buildings = content_reader_key_names::buildings::parse(source.as_bytes(), &KeyNames::default()).unwrap();
        assert!(!Content::new(&buildings).building.unwrap().in_states);
    }
}
