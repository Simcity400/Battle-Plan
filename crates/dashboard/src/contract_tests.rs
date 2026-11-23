
#[test]
fn the_frontend_command_budget_matches_the_reply_transport() {
    assert_eq!(
        dashboard_core::protocol::MAX_MESSAGE_BYTES,
        dashboard_core::transport::MAX_REPLY_BYTES
    );
    assert!(crate::typescript_contract().contains(&format!(
        "export const MAX_MESSAGE_BYTES = {};",
        dashboard_core::protocol::MAX_MESSAGE_BYTES
    )));
}

#[test]
fn the_contract_names_the_refusals_a_reader_may_try_again() {
    use dashboard_core::transport::{BROWSER_BRIDGE_BUSY, CAPACITY_REACHED, MAP_CONTROLS_BUSY, NATIVE_QUEUE_FULL, TRANSIENT_ERROR_NAMES, TRANSIENT_ERRORS, transient_error};
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
    let table = crate::request_fields();
    let mut wrong = Vec::new();
    for product in crate::products::PRODUCTS {
        let walk = (product.params)();
        wrong.extend(walk.problems(product.fields).into_iter().map(|problem| format!("{}: {problem}", product.key)));
        assert!(!(product.read_hour && product.fields.iter().any(|(path, _)| *path == "hour")), "{}: an hourly product's hour is its own", product.key);
        assert!(table.contains_key(product.key), "{} is missing from the contract", product.key);
    }
    let series = crate::series_walk();
    wrong.extend(series.problems(dashboard_query::series::FIELDS).into_iter().map(|problem| format!("series: {problem}")));
    for key in crate::answer::SERIES_PRODUCTS {
        assert!(table.contains_key(*key), "{key} is missing from the contract");
    }
    for (head, fields, walk) in dashboard_players::products::player_apm::head::HEADS {
        wrong.extend(walk().problems(fields).into_iter().map(|problem| format!("head {head}: {problem}")));
        assert!(table.contains_key(&format!("head:{head}")));
    }
    assert!(wrong.is_empty(), "every request field that can carry a number declares its role:\n{}", wrong.join("\n"));
    let contract = crate::typescript_contract();
    for (key, fields) in &table {
        let roles = fields.iter().map(|(path, role)| format!("{path:?}: {role:?}")).collect::<Vec<_>>().join(", ");
        let row = if roles.is_empty() { format!("  {key:?}: {{}},") } else { format!("  {key:?}: {{ {roles} }},") };
        assert!(contract.contains(&row), "the contract carries {key}'s roles");
    }
    assert_eq!(table["production-line-breakdown"], [("hour", "bucket"), ("within[].firstHour", "start"), ("within[].lastHour", "end")]);
    assert_eq!(table["equipment-produced"], [("rankHour", "read")]);
    assert_eq!(table["series-state"].iter().find(|(path, _)| *path == "reads[].hour"), Some(&("reads[].hour", "read")));
    assert!(table["research-breakdown"].contains(&("parts[].firstObservedHour", "read")));
    assert!(table["series-tiles"].contains(&("family.membership[].firstHour", "start")));
    let paths = crate::request_paths();
    assert_eq!(paths.keys().collect::<Vec<_>>(), table.keys().collect::<Vec<_>>(), "every product's field names are in the contract");
    assert_eq!(paths["series-state"].get("reads[].country"), Some(&"any"));
    assert_eq!(paths["series-state"].get("reads[].tag"), None, "a series read names its country, never a tag field");
    assert_eq!(paths["series-tiles"].get("binHours"), Some(&"leaf"), "the eAPM and army tile asks are walked with the series");
}

