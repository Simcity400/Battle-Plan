//! Lane-owned production module boundary for army_battleplans_depth_live.
//!
//! The shared DLL coordinator can register this module later and feed it the
//! same live order observations used by the base army-orders collector. This
//! file does not edit or depend on shared collector entrypoints.

#[path = "army_battleplans_depth_collector.rs"]
mod collector;
#[path = "army_battleplans_depth_compact.rs"]
mod compact;

pub use collector::{
    RawOrderObservation, ARMY_OFF_ORDERS_VEC, ARMY_VTABLE_RVA, ORDER_OFF_ARMY_BACKREF,
    ORDER_OFF_TYPE, ORDER_OFF_TYPE5_STATES_BEGIN, ORDER_OFF_TYPE5_STATES_SIZE, ORDER_VTABLE_RVA,
};

pub const DOMAIN_ID: u16 = compact::DOMAIN_ARMY_BATTLEPLANS_DEPTH;
pub const DOMAIN_NAME: &str = "army_battleplans_depth_live";
pub const WIRE_NAME: &str = "army_battleplans_depth_live";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainRegistration {
    pub domain_id: u16,
    pub canonical_name: &'static str,
    pub wire_name: &'static str,
    pub requires_data_record: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveDepthModuleInput {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub observed_army_rows: u32,
    pub observed_order_rows: u32,
    pub orders: Vec<RawOrderObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedDepthRecord {
    pub domain_id: u16,
    pub payload: Vec<u8>,
    pub emitted_rows: u32,
    pub type5_exact_rows: u32,
    pub type5_non_exact_omitted_rows: u32,
    pub non_type5_fixture_blocked_rows: u32,
    pub execution_fixture_blocked_rows: u32,
}

pub trait CompactDomainSink {
    fn emit_domain_data(&mut self, domain_id: u16, payload: &[u8]);
}

pub fn registration() -> DomainRegistration {
    DomainRegistration {
        domain_id: DOMAIN_ID,
        canonical_name: DOMAIN_NAME,
        wire_name: WIRE_NAME,
        requires_data_record: true,
    }
}

pub fn collect_and_encode_depth_record(input: &LiveDepthModuleInput) -> EncodedDepthRecord {
    let extracted = collector::extract_validated_depth_subset(&input.orders);
    let rows = extracted
        .rows
        .iter()
        .map(|row| compact::BattleplanDepthRow {
            army_id: row.army_id,
            generation: row.generation,
            deleted: row.deleted,
            slot_index: row.slot_index,
            order_type: row.order_type,
            target_states: row.target_states.clone(),
            target_declared_count: row.target_declared_count,
            target_status: row.target_status,
            geometry_status: row.geometry_status,
            execution_status: row.execution_status,
        })
        .collect::<Vec<_>>();
    let emitted_rows = rows.len().min(u32::MAX as usize) as u32;
    let payload = compact::encode_depth_payload(
        &rows,
        input.elapsed_wall_ms,
        input.status_flags,
        emitted_rows,
        input.observed_army_rows,
        input.observed_order_rows,
    );
    EncodedDepthRecord {
        domain_id: DOMAIN_ID,
        payload,
        emitted_rows,
        type5_exact_rows: extracted.type5_exact_rows,
        type5_non_exact_omitted_rows: extracted.type5_non_exact_omitted_rows,
        non_type5_fixture_blocked_rows: extracted.non_type5_fixture_blocked_rows,
        execution_fixture_blocked_rows: extracted.execution_fixture_blocked_rows,
    }
}

pub fn collect_and_emit_depth_record(input: &LiveDepthModuleInput, sink: &mut dyn CompactDomainSink) {
    let record = collect_and_encode_depth_record(input);
    sink.emit_domain_data(record.domain_id, &record.payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestSink {
        domain_id: Option<u16>,
        payload: Vec<u8>,
    }

    impl CompactDomainSink for TestSink {
        fn emit_domain_data(&mut self, domain_id: u16, payload: &[u8]) {
            self.domain_id = Some(domain_id);
            self.payload = payload.to_vec();
        }
    }

    #[test]
    fn registration_matches_domain_registry_assignment() {
        let reg = registration();
        assert_eq!(reg.domain_id, 54);
        assert_eq!(reg.canonical_name, "army_battleplans_depth_live");
        assert_eq!(reg.wire_name, "army_battleplans_depth_live");
        assert!(reg.requires_data_record);
    }

    #[test]
    fn encodes_domain54_payload_without_domain27_stream_conversion() {
        let input = LiveDepthModuleInput {
            elapsed_wall_ms: 77,
            status_flags: 0,
            observed_army_rows: 2,
            observed_order_rows: 2,
            orders: vec![
                RawOrderObservation {
                    army_id: 10,
                    generation: 1,
                    deleted: false,
                    slot_index: 0,
                    order_type: 5,
                    target_states: vec![44, 45],
                    target_declared_count: Some(2),
                    target_status: collector::STATUS_EXACT,
                },
                RawOrderObservation {
                    army_id: 11,
                    generation: 1,
                    deleted: false,
                    slot_index: 0,
                    order_type: 2,
                    target_states: Vec::new(),
                    target_declared_count: None,
                    target_status: collector::STATUS_UNSUPPORTED,
                },
            ],
        };
        let record = collect_and_encode_depth_record(&input);
        assert_eq!(record.domain_id, 54);
        assert_eq!(record.emitted_rows, 2);
        assert_eq!(record.type5_exact_rows, 1);
        assert_eq!(record.non_type5_fixture_blocked_rows, 1);
        assert_eq!(record.execution_fixture_blocked_rows, 2);

        let mut summary = compact::GateSummary::default();
        compact::decode_depth_payload(0, &record.payload, &mut summary);
        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(summary.totals.rows, 2);
        assert_eq!(summary.totals.target_rows, 2);
        assert_eq!(summary.totals.unsupported_geometry_rows, 1);
        assert_eq!(summary.totals.execution_unknown_rows, 2);
    }

    #[test]
    fn sink_receives_domain54_payload_and_non_exact_type5_is_omitted() {
        let input = LiveDepthModuleInput {
            elapsed_wall_ms: 88,
            status_flags: 0,
            observed_army_rows: 1,
            observed_order_rows: 1,
            orders: vec![RawOrderObservation {
                army_id: 10,
                generation: 1,
                deleted: false,
                slot_index: 0,
                order_type: 5,
                target_states: Vec::new(),
                target_declared_count: Some(0),
                target_status: collector::STATUS_UNSUPPORTED,
            }],
        };
        let record = collect_and_encode_depth_record(&input);
        assert_eq!(record.emitted_rows, 0);
        assert_eq!(record.type5_non_exact_omitted_rows, 1);

        let mut sink = TestSink::default();
        collect_and_emit_depth_record(&input, &mut sink);
        assert_eq!(sink.domain_id, Some(54));
        let mut summary = compact::GateSummary::default();
        compact::decode_depth_payload(0, &sink.payload, &mut summary);
        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(summary.totals.rows, 0);
    }
}
