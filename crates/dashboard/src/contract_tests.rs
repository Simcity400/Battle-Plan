
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
