use super::*;

#[test]
fn every_rows_at_an_hour_product_is_registered_moved_and_shown_as_a_panel() {
    for entry in crate::products::AT_HOUR {
        let key = entry.registered.key;
        assert!(crate::products::find(key).is_some(), "{key} answers requests");
        assert!(crate::products::MOVED.contains(&key), "{key} is answered through the query layer");
    }
    assert!(crate::products::find(super::super::army::TemplateRoster::KEY).is_some());
}
