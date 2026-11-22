
#[test]
fn the_frontend_command_budget_matches_the_reply_transport() {
    assert_eq!(
        crate::protocol::MAX_MESSAGE_BYTES,
        crate::transport::MAX_REPLY_BYTES
    );
    assert!(crate::typescript_contract().contains(&format!(
        "export const MAX_MESSAGE_BYTES = {};",
        crate::protocol::MAX_MESSAGE_BYTES
    )));
}

#[test]
fn the_contract_names_the_refusals_a_reader_may_try_again() {
    use crate::transport::{
        BROWSER_BRIDGE_BUSY, CAPACITY_REACHED, MAP_CONTROLS_BUSY, NATIVE_QUEUE_FULL,
        TRANSIENT_ERROR_NAMES, TRANSIENT_ERRORS, transient_error,
    };
    let contract = crate::typescript_contract();
    for (name, message) in TRANSIENT_ERROR_NAMES {
        assert!(contract.contains(&format!("export const {name} = {message:?};")));
    }
    assert!(contract.contains(
        "export const TRANSIENT_ERRORS = [CAPACITY_REACHED, BROWSER_BRIDGE_BUSY, NATIVE_QUEUE_FULL, MAP_CONTROLS_BUSY] as const;"
    ));
    assert_eq!(
        TRANSIENT_ERRORS,
        [
            CAPACITY_REACHED,
            BROWSER_BRIDGE_BUSY,
            NATIVE_QUEUE_FULL,
            MAP_CONTROLS_BUSY
        ]
    );
    for refusal in TRANSIENT_ERRORS {
        assert!(transient_error(refusal), "{refusal}");
        assert!(transient_error(&format!("{refusal}. Retry this view.")));
    }
    for answer in [
        "Choose the game and user folders in Settings first",
        "Dashboard request cancelled",
        "Source identity exhausted",
        "Too many native map views",
        "",
    ] {
        assert!(!transient_error(answer), "{answer}");
    }
}

#[test]
fn every_product_request_field_declares_its_hour_role_and_the_contract_carries_it() {
    use dashboard_query::fields::field_head;
    use std::collections::BTreeSet;
    const OPAQUE: &[&str] = &["faction-state"];
    let heads = |fields: crate::products::Fields| fields.iter().map(|(path, _)| field_head(path)).collect::<BTreeSet<_>>();
    let table = crate::request_fields();
    let mut wrong = Vec::new();
    for product in crate::products::PRODUCTS {
        let declared = heads(product.fields);
        match (product.params)() {
            Some(asked) if declared != asked.iter().copied().collect::<BTreeSet<_>>() => wrong.push(format!("{}: declares {declared:?}, asks {asked:?}", product.key)),
            None if !OPAQUE.contains(&product.key) => wrong.push(format!("{}: its request fields cannot be listed", product.key)),
            _ => {}
        }
        assert!(!(product.read_hour && declared.contains("hour")), "{}: an hourly product's hour is its own", product.key);
        assert!(table.contains_key(product.key), "{} is missing from the contract", product.key);
    }
    assert!(wrong.is_empty(), "every request field declares its role:\n{}", wrong.join("\n"));
    let series = heads(dashboard_query::series::FIELDS);
    for asked in [dashboard_query::series::params_fields(), dashboard_query::tile::params_fields()] {
        let asked = asked.expect("series requests list their fields");
        assert!(asked.iter().all(|field| series.contains(field)), "series fields {asked:?} declare their roles");
    }
    for key in crate::answer::SERIES_PRODUCTS {
        assert!(table.contains_key(*key), "{key} is missing from the contract");
    }
    for (head, fields, asked) in crate::products::player_apm::head::HEADS {
        assert_eq!(heads(fields), asked().expect("a head lists its fields").iter().copied().collect(), "head {head}");
        assert!(table.contains_key(&format!("head:{head}")));
    }
    let contract = crate::typescript_contract();
    for (key, fields) in &table {
        let roles = fields.iter().map(|(path, role)| format!("{path:?}: {role:?}")).collect::<Vec<_>>().join(", ");
        let row = if roles.is_empty() { format!("  {key:?}: {{}},") } else { format!("  {key:?}: {{ {roles} }},") };
        assert!(contract.contains(&row), "the contract carries {key}'s roles");
    }
    assert_eq!(table["production-line-breakdown"], [("hour", "bucket")]);
    assert_eq!(table["equipment-produced"], [("rankHour", "read")]);
    assert_eq!(table["series-state"].iter().find(|(path, _)| *path == "reads[].hour"), Some(&("reads[].hour", "read")));
}
