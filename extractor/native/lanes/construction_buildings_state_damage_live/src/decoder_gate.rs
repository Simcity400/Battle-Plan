use crate::{
    decode_lane_frame, gate_decoded_frame, GateReport, LaneFrame, DOMAIN_ID, DOMAIN_NAME,
    SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
};

pub const DECODER_MODULE_NAME: &str = "construction_buildings_state_damage_live.decoder_gate";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecoderSummary {
    pub domain_id: u16,
    pub domain_name: &'static str,
    pub construction_rows: usize,
    pub railway_construction_rows: usize,
    pub construction_rows_with_target_state: usize,
    pub target_state_unavailable_badged: bool,
    pub building_rows: usize,
    pub damaged_building_rows: usize,
    pub resource_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDomain48 {
    pub frame: LaneFrame,
    pub gate: GateReport,
    pub summary: DecoderSummary,
}

pub fn decode_domain48_compact(bytes: &[u8]) -> Result<DecodedDomain48, String> {
    let frame = decode_lane_frame(bytes)?;
    let gate = gate_decoded_frame(&frame);
    let summary = summarize_frame(&frame, &gate);
    Ok(DecodedDomain48 {
        frame,
        gate,
        summary,
    })
}

pub fn summarize_frame(frame: &LaneFrame, gate: &GateReport) -> DecoderSummary {
    DecoderSummary {
        domain_id: DOMAIN_ID,
        domain_name: DOMAIN_NAME,
        construction_rows: frame.construction_rows.len(),
        railway_construction_rows: frame.railway_construction_rows.as_ref().map_or(0, Vec::len),
        construction_rows_with_target_state: gate.construction_rows_with_target_state,
        target_state_unavailable_badged: frame.source_flags
            & SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
            != 0,
        building_rows: frame.building_rows.len(),
        damaged_building_rows: gate.damaged_building_rows,
        resource_rows: frame.resource_rows.len(),
    }
}
