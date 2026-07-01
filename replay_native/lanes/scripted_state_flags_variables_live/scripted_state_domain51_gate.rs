#[path = "scripted_state_compact.rs"]
pub mod compact;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain51GateVerdict {
    pub accepted: bool,
    pub partial_failure: bool,
    pub unresolved_delayed_events: bool,
    pub unresolved_scripted_gui: bool,
    pub summary: compact::Domain51GateSummary,
}

pub fn gate_domain51_payload(offset: usize, payload: &[u8]) -> Domain51GateVerdict {
    let summary = compact::decode_payload(offset, payload);
    let partial_failure = summary.status_flags_or
        & (compact::STATUS_PARTIAL_DATA | compact::STATUS_SOURCE_UNAVAILABLE)
        != 0;
    let unresolved_delayed_events =
        summary.status_flags_or & compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED != 0;
    let unresolved_scripted_gui =
        summary.status_flags_or & compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED != 0;

    Domain51GateVerdict {
        accepted: summary.parse_errors.is_empty(),
        partial_failure,
        unresolved_delayed_events,
        unresolved_scripted_gui,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_opaque_roots_are_not_partial_failures() {
        let rows = vec![
            compact::ScriptedStateRow {
                scope_kind: compact::SCOPE_GLOBAL,
                owner_id: 0,
                value_kind: compact::VALUE_DELAYED_EVENT_OPAQUE,
                key: Some("root_unresolved:delayed_events".to_string()),
                token_id: None,
                value_raw: None,
                date_hour_raw: None,
                deleted: false,
            },
            compact::ScriptedStateRow {
                scope_kind: compact::SCOPE_GLOBAL,
                owner_id: 0,
                value_kind: compact::VALUE_SCRIPTED_GUI_OPAQUE,
                key: Some("root_unresolved:scripted_gui_open_state".to_string()),
                token_id: None,
                value_raw: None,
                date_hour_raw: None,
                deleted: false,
            },
        ];
        let header = compact::ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
                | compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED,
            changed_rows: rows.len() as u32,
            observed_scopes: 2,
            measured_scopes: 0,
            rejected_scopes: 2,
        };
        let payload = compact::encode_payload(&header, &rows);
        let verdict = gate_domain51_payload(0, &payload);

        assert!(verdict.accepted, "{:?}", verdict.summary.parse_errors);
        assert!(!verdict.partial_failure);
        assert!(verdict.unresolved_delayed_events);
        assert!(verdict.unresolved_scripted_gui);
    }

    #[test]
    fn validated_scope_partial_data_is_a_partial_failure() {
        let header = compact::ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: compact::STATUS_PARTIAL_DATA,
            changed_rows: 0,
            observed_scopes: 1,
            measured_scopes: 0,
            rejected_scopes: 1,
        };
        let payload = compact::encode_payload(&header, &[]);
        let verdict = gate_domain51_payload(0, &payload);

        assert!(verdict.accepted, "{:?}", verdict.summary.parse_errors);
        assert!(verdict.partial_failure);
        assert!(!verdict.unresolved_delayed_events);
        assert!(!verdict.unresolved_scripted_gui);
    }

    #[test]
    fn malformed_payload_is_rejected() {
        let verdict = gate_domain51_payload(8, &[1, 2, 3]);
        assert!(!verdict.accepted);
        assert!(!verdict.summary.parse_errors.is_empty());
    }
}
