//! Domain 81 `rail_replay_state_live` payload contract.
//!
//! The payload is a pointer-free full snapshot of the exact data consumed by
//! the pinned a729 `CRailwayGfx` rebuild: one engine-selected world anchor per
//! rail province and the ordered `(neighbor_province_id, edge_level)` rows from
//! that province's `RailwayInfo`. The renderer joins reciprocal rows and uses
//! `min(level_a, level_b)`; the wire deliberately preserves both directed
//! values so that law can be replayed mechanically.

use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 81;
pub const DOMAIN_NAME: &str = "rail_replay_state_live";
pub const PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"RRL1");
pub const PAYLOAD_VERSION_V1: u16 = 1;
pub const PAYLOAD_VERSION_V2: u16 = 2;
pub const PAYLOAD_VERSION_V3: u16 = 3;
/// V3 replaces the invalid UI-owned cached-frame source with raw simulation
/// rows. V1/V2 remain decode-only compatibility formats.
pub const PAYLOAD_VERSION: u16 = PAYLOAD_VERSION_V3;
pub const HEADER_LEN: usize = 40;
pub const ROW_LEN: usize = 32;
pub const ROW_KIND_ANCHOR: u8 = 1;
pub const ROW_KIND_NEIGHBOR: u8 = 2;
pub const ROW_KIND_HUB_FRAME: u8 = 3;
pub const ROW_KIND_HUB_SIMULATION: u8 = 4;
const ROW_KIND_HUB_SIMULATION_CONTINUATION: u8 = 5;
pub const ROW_FLAG_ACTIVE_TERMINAL: u8 = 1;
pub const ROW_FLAG_HUB_SUBTYPE: u8 = 1;
pub const ROW_FLAG_HUB_NAVAL_OVERLAY_VISIBLE: u8 = 1 << 1;
pub const ROW_FLAG_HUB_KNOWN: u8 = ROW_FLAG_HUB_SUBTYPE | ROW_FLAG_HUB_NAVAL_OVERLAY_VISIBLE;
/// A v1 fallback explicitly distinguishes unavailable from a measured-empty
/// hub set. The promoted collector emits this only when its complete
/// authenticated owner-vector double-read cannot be obtained.
pub const PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE: u32 = 1;
/// Historical schema-v2 presentation-cache completeness bit. V2 is
/// decode-only and is not a production authority.
pub const PAYLOAD_FLAG_HUB_FRAMES_COMPLETE: u32 = 1 << 1;
/// Schema v3 contains a complete, pointer-free snapshot of the raw simulation
/// inputs required by the pinned supply-node rendering law.
pub const PAYLOAD_FLAG_HUB_SIMULATION_COMPLETE: u32 = 1 << 2;
/// Frozen Wave 2 exact-ten authority for D81. These hashes are duplicated in
/// `D81_PROMOTION_MANIFEST_20260720.json`; changing any input requires a new
/// reviewed promotion manifest rather than silently moving the authority.
pub const D81_EXACT10_REEXPORT_RECEIPT_SHA256: &str =
    "00108a3ce3a5419d3ce9c161af810f9540e711adbb703d7fb44e05ad95f1e4f4";
pub const D81_EXACT10_POOL_RECEIPT_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const D81_PROMOTION_MANIFEST_SHA256: &str =
    "32388518a63eaa22ea6290e82fb3677a5515039e937334ebc2c2d26a0c0bc6b1";
pub const D81_RAILWAY_CARRIER_MANIFEST_SHA256: &str =
    "dd19cb97391dcaa77b8eb4c2f9980d4b86fa1e703342e5e9843b76995a9edb31";
pub const D81_OWNER_LOADER_MANIFEST_SHA256: &str =
    "0fd6c56d702f010e46503a9d566e9a174c04505c7f6a30ce46ae48b82836275c";
pub const D81_OWNER_ROUTE_MANIFEST_SHA256: &str =
    "990aefa465b125acd5ac9d894c62a38456bcd31ab531eba3b10673bc50255c4a";
pub const D81_VISUAL_LAW_MANIFEST_SHA256: &str =
    "492e712cd283a75b9b6357eb42f4bc98137d1ab138cfa857164f8d353f675c28";
pub const D81_OWNER_SUPPLEMENT_PCODE_MANIFEST_SHA256: &str =
    "d50e88d325dc098c7e676de879563e99de6b689d6ab58e5cb094320b1c0903e4";
pub const D81_OWNER_SUPPLEMENT_NO_PCODE_MANIFEST_SHA256: &str =
    "bc0a7c92bb400a5505b92ea4c0d490b14f97bb82593bda8ca6c246d7347d8840";

/// V2 cached-frame capture is retained solely for decoding historical local
/// evidence. Its source is UI-owned and is not a production authority.
pub const HUB_FRAME_OWNER_ROUTE_PROMOTED: bool = false;
pub const HUB_FRAME_V2_ENCODER_PROMOTED: bool = false;
pub const HUB_FRAME_V2_FOLD_PROMOTED: bool = false;
pub const HUB_FRAME_V2_AUTHORITY_PROMOTED: bool = false;
pub const HUB_FRAME_LIVE_COLLECTOR_PROMOTED: bool = false;
pub const MAX_PROVINCES: usize = 20_000;
pub const MAX_NEIGHBORS_PER_PROVINCE: usize = 64;
pub const MAX_RAIL_ROWS: usize = MAX_PROVINCES * (MAX_NEIGHBORS_PER_PROVINCE + 1);
pub const MAX_HUB_FRAMES: usize = MAX_PROVINCES * 2;
/// The sparse per-country supply tables are bounded independently from the
/// province carrier. Each logical simulation row consumes two physical
/// 32-byte rows so the existing row ABI stays fixed.
pub const MAX_HUB_SIMULATION_ROWS: usize = MAX_PROVINCES * 64;
pub const MAX_ROWS: usize = MAX_RAIL_ROWS + MAX_HUB_SIMULATION_ROWS * 2;
pub const MAX_RAIL_LEVEL: u32 = 5;
pub const MAX_HUB_SUBTYPE: u8 = 1;
pub const MIN_HUB_BACKGROUND_FRAME: u8 = 1;
pub const MAX_HUB_BACKGROUND_FRAME: u8 = 5;
pub const MIN_HUB_COMMON_ICON_FRAME: u8 = 1;
pub const MAX_HUB_COMMON_ICON_FRAME: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RailNeighborState {
    pub province_id: u32,
    pub edge_level: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailProvinceState {
    pub province_id: u32,
    pub active_terminal: bool,
    /// 0 selects `CProvince+0x14e4`; 1 selects `CProvince+0x14f0`.
    pub anchor_variant: u8,
    /// Raw IEEE-754 f32 bits in engine x/y/z order.
    pub anchor_bits: [u32; 3],
    /// Exact engine order. No numeric ordering is imposed because the renderer
    /// consumes producer order; only uniqueness is required.
    pub neighbors: Vec<RailNeighborState>,
}

/// Historical pointer-free schema-v2 presentation-cache state. Retained for
/// decoding local evidence only; production v3 does not capture these values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RailHubFrameState {
    pub province_id: u32,
    pub subtype: u8,
    pub background_button_frame: u8,
    pub common_icon_frame: u8,
    pub naval_overlay_visible: bool,
}

/// Raw simulation inputs for one `(pov country, province, subtype)` identity.
///
/// No selected province, effective render country, UI cache, or derived frame
/// is present. The replay consumer supplies viewer inputs independently and
/// applies the pinned rendering law mechanically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RailHubSimulationState {
    pub pov_country_id: u32,
    pub province_id: u32,
    pub province_owner_country_id: u32,
    pub subtype: u8,
    pub route_class: u8,
    pub node_kind: u32,
    pub node_flag: u8,
    pub available_raw: i64,
    pub consumed_raw: i64,
    pub foreign_max_raw: i64,
    pub foreign_route: bool,
    pub fallback_record: bool,
}

/// decode-only v2 compatibility and pinned-law tests; v3 does not capture it.
pub fn common_icon_frame_from_selector(selector: u32) -> Option<u8> {
    match selector {
        0 => Some(1),
        1 => Some(3),
        2 => Some(2),
        3 => Some(4),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRailReplayPayload {
    pub payload_version: u16,
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub payload_flags: u32,
    pub observed_provinces: u32,
    pub observed_neighbors: u32,
    pub provinces: Vec<RailProvinceState>,
    pub hub_frames_complete: bool,
    pub hub_frames: Vec<RailHubFrameState>,
    pub hub_simulation_complete: bool,
    pub hub_simulation_rows: Vec<RailHubSimulationState>,
}

pub fn encode_payload(
    provinces: &[RailProvinceState],
    elapsed_wall_ms: u64,
    status_flags: u32,
    payload_flags: u32,
    observed_provinces: u32,
    observed_neighbors: u32,
) -> Result<Vec<u8>, String> {
    if payload_flags != PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE {
        return Err("rail replay v1 must explicitly badge hub frames unavailable".into());
    }
    encode_payload_inner(
        PAYLOAD_VERSION_V1,
        provinces,
        &[],
        &[],
        elapsed_wall_ms,
        status_flags,
        payload_flags,
        observed_provinces,
        observed_neighbors,
    )
}

/// Encode an authenticated, complete D81 snapshot. This function is only the
/// pointer-free carrier seam. The collector calls it only when its complete
/// authenticated double-read returns `Some`, including measured empty.
pub fn encode_payload_with_hub_frames(
    provinces: &[RailProvinceState],
    hub_frames: &[RailHubFrameState],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_provinces: u32,
    observed_neighbors: u32,
) -> Result<Vec<u8>, String> {
    if !HUB_FRAME_V2_ENCODER_PROMOTED {
        return Err("rail replay v2 encoder is not admitted by the D81 promotion manifest".into());
    }
    encode_payload_inner(
        PAYLOAD_VERSION_V2,
        provinces,
        hub_frames,
        &[],
        elapsed_wall_ms,
        status_flags,
        PAYLOAD_FLAG_HUB_FRAMES_COMPLETE,
        observed_provinces,
        observed_neighbors,
    )
}

/// Encode the production D81 v3 simulation-source snapshot.
///
/// A successful call always carries a complete simulation row set, including
/// the measured-empty case. Acquisition failure is represented by the
/// enclosing domain's H4DG terminal outcome, never by a partial D81 payload.
pub fn encode_payload_with_hub_simulation(
    provinces: &[RailProvinceState],
    rows: &[RailHubSimulationState],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_provinces: u32,
    observed_neighbors: u32,
) -> Result<Vec<u8>, String> {
    encode_payload_inner(
        PAYLOAD_VERSION_V3,
        provinces,
        &[],
        rows,
        elapsed_wall_ms,
        status_flags,
        PAYLOAD_FLAG_HUB_SIMULATION_COMPLETE,
        observed_provinces,
        observed_neighbors,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_payload_inner(
    payload_version: u16,
    provinces: &[RailProvinceState],
    hub_frames: &[RailHubFrameState],
    hub_simulation_rows: &[RailHubSimulationState],
    elapsed_wall_ms: u64,
    status_flags: u32,
    payload_flags: u32,
    observed_provinces: u32,
    observed_neighbors: u32,
) -> Result<Vec<u8>, String> {
    validate(
        payload_version,
        provinces,
        hub_frames,
        hub_simulation_rows,
        payload_flags,
        observed_provinces,
        observed_neighbors,
    )?;
    let rail_row_count = provinces
        .len()
        .checked_add(provinces.iter().map(|p| p.neighbors.len()).sum::<usize>())
        .ok_or_else(|| "rail replay row count overflow".to_string())?;
    let row_count = rail_row_count
        .checked_add(hub_frames.len())
        .and_then(|count| {
            hub_simulation_rows
                .len()
                .checked_mul(2)
                .and_then(|simulation_rows| count.checked_add(simulation_rows))
        })
        .ok_or_else(|| "rail replay hub row count overflow".to_string())?;
    let capacity = HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(ROW_LEN)
                .ok_or_else(|| "rail replay row bytes overflow".to_string())?,
        )
        .ok_or_else(|| "rail replay payload bytes overflow".to_string())?;
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
    out.extend_from_slice(&payload_version.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&observed_provinces.to_le_bytes());
    out.extend_from_slice(&observed_neighbors.to_le_bytes());
    out.extend_from_slice(&payload_flags.to_le_bytes());
    out.extend_from_slice(&(row_count as u32).to_le_bytes());
    for province in provinces {
        push_row(
            &mut out,
            ROW_KIND_ANCHOR,
            if province.active_terminal {
                ROW_FLAG_ACTIVE_TERMINAL
            } else {
                0
            },
            0,
            province.province_id,
            0,
            u32::from(province.anchor_variant),
            province.anchor_bits,
        );
        for (ordinal, neighbor) in province.neighbors.iter().enumerate() {
            push_row(
                &mut out,
                ROW_KIND_NEIGHBOR,
                0,
                ordinal as u16,
                province.province_id,
                neighbor.province_id,
                neighbor.edge_level,
                [0; 3],
            );
        }
    }
    for hub in hub_frames {
        let flags = (hub.subtype & ROW_FLAG_HUB_SUBTYPE)
            | if hub.naval_overlay_visible {
                ROW_FLAG_HUB_NAVAL_OVERLAY_VISIBLE
            } else {
                0
            };
        push_row(
            &mut out,
            ROW_KIND_HUB_FRAME,
            flags,
            0,
            hub.province_id,
            0,
            u32::from(hub.background_button_frame),
            [u32::from(hub.common_icon_frame), 0, 0],
        );
    }
    for (ordinal, row) in hub_simulation_rows.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| "rail replay simulation ordinal exceeds u32".to_string())?;
        out.push(ROW_KIND_HUB_SIMULATION);
        out.push(u8::from(row.foreign_route) | (u8::from(row.fallback_record) << 1));
        out.push(row.subtype);
        out.push(row.route_class);
        out.extend_from_slice(&row.pov_country_id.to_le_bytes());
        out.extend_from_slice(&row.province_id.to_le_bytes());
        out.extend_from_slice(&row.province_owner_country_id.to_le_bytes());
        out.extend_from_slice(&row.node_kind.to_le_bytes());
        out.push(row.node_flag);
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&ordinal.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        debug_assert_eq!((out.len() - HEADER_LEN) % ROW_LEN, 0);

        out.push(ROW_KIND_HUB_SIMULATION_CONTINUATION);
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&ordinal.to_le_bytes());
        out.extend_from_slice(&row.available_raw.to_le_bytes());
        out.extend_from_slice(&row.consumed_raw.to_le_bytes());
        out.extend_from_slice(&row.foreign_max_raw.to_le_bytes());
        debug_assert_eq!((out.len() - HEADER_LEN) % ROW_LEN, 0);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    out: &mut Vec<u8>,
    kind: u8,
    flags: u8,
    ordinal: u16,
    province_id: u32,
    related_id: u32,
    value: u32,
    bits: [u32; 3],
) {
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&ordinal.to_le_bytes());
    out.extend_from_slice(&province_id.to_le_bytes());
    out.extend_from_slice(&related_id.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
    for value in bits {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
}

pub fn decode_payload(payload: &[u8]) -> Result<DecodedRailReplayPayload, String> {
    if payload.len() < HEADER_LEN {
        return Err("rail replay payload truncated before header".into());
    }
    if read_u32(payload, 0)? != PAYLOAD_MAGIC {
        return Err("rail replay payload magic is not RRL1".into());
    }
    let payload_version = read_u16(payload, 4)?;
    if !matches!(
        payload_version,
        PAYLOAD_VERSION_V1 | PAYLOAD_VERSION_V2 | PAYLOAD_VERSION_V3
    ) {
        return Err("rail replay payload version is unsupported".into());
    }
    let header_len = read_u16(payload, 6)? as usize;
    let row_len = read_u16(payload, 8)? as usize;
    if header_len != HEADER_LEN || row_len != ROW_LEN || read_u16(payload, 10)? != 0 {
        return Err("rail replay schema header lengths/reserved field are invalid".into());
    }
    let status_flags = read_u32(payload, 12)?;
    let elapsed_wall_ms = read_u64(payload, 16)?;
    let observed_provinces = read_u32(payload, 24)?;
    let observed_neighbors = read_u32(payload, 28)?;
    let payload_flags = read_u32(payload, 32)?;
    let row_count = read_u32(payload, 36)? as usize;
    if row_count > MAX_ROWS {
        return Err(format!(
            "rail replay row count {row_count} exceeds {MAX_ROWS}"
        ));
    }
    let expected = HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(ROW_LEN)
                .ok_or_else(|| "rail replay row bytes overflow".to_string())?,
        )
        .ok_or_else(|| "rail replay payload bytes overflow".to_string())?;
    if payload.len() != expected {
        return Err(format!(
            "rail replay payload length {} does not exactly cover {row_count} rows ({expected})",
            payload.len()
        ));
    }
    let mut provinces = Vec::new();
    let mut hub_frames = Vec::new();
    let mut hub_simulation_rows = Vec::new();
    let mut hub_rows_started = false;
    let mut index = 0usize;
    while index < row_count {
        let base = HEADER_LEN + index * ROW_LEN;
        let kind = payload[base];
        let flags = payload[base + 1];
        let ordinal = read_u16(payload, base + 2)?;
        let province_id = read_u32(payload, base + 4)?;
        let related_id = read_u32(payload, base + 8)?;
        let value = read_u32(payload, base + 12)?;
        let bits = [
            read_u32(payload, base + 16)?,
            read_u32(payload, base + 20)?,
            read_u32(payload, base + 24)?,
        ];
        if read_u32(payload, base + 28)? != 0 {
            return Err(format!("rail replay row {index} has nonzero reserved tail"));
        }
        match kind {
            ROW_KIND_ANCHOR => {
                if hub_rows_started {
                    return Err(format!("rail replay anchor row {index} follows hub rows"));
                }
                if ordinal != 0
                    || related_id != 0
                    || flags & !ROW_FLAG_ACTIVE_TERMINAL != 0
                    || value > 1
                {
                    return Err(format!(
                        "rail replay anchor row {index} has invalid fixed fields"
                    ));
                }
                provinces.push(RailProvinceState {
                    province_id,
                    active_terminal: flags != 0,
                    anchor_variant: value as u8,
                    anchor_bits: bits,
                    neighbors: Vec::new(),
                });
            }
            ROW_KIND_NEIGHBOR => {
                if hub_rows_started {
                    return Err(format!("rail replay neighbor row {index} follows hub rows"));
                }
                let Some(parent) = provinces.last_mut() else {
                    return Err(format!(
                        "rail replay neighbor row {index} precedes its anchor"
                    ));
                };
                if flags != 0
                    || province_id != parent.province_id
                    || ordinal as usize != parent.neighbors.len()
                    || bits != [0; 3]
                {
                    return Err(format!(
                        "rail replay neighbor row {index} has invalid grouping/fixed fields"
                    ));
                }
                parent.neighbors.push(RailNeighborState {
                    province_id: related_id,
                    edge_level: value,
                });
            }
            ROW_KIND_HUB_FRAME => {
                hub_rows_started = true;
                if payload_version != PAYLOAD_VERSION_V2
                    || ordinal != 0
                    || related_id != 0
                    || flags & !ROW_FLAG_HUB_KNOWN != 0
                    || !(u32::from(MIN_HUB_BACKGROUND_FRAME)..=u32::from(MAX_HUB_BACKGROUND_FRAME))
                        .contains(&value)
                    || !(u32::from(MIN_HUB_COMMON_ICON_FRAME)
                        ..=u32::from(MAX_HUB_COMMON_ICON_FRAME))
                        .contains(&bits[0])
                    || bits[1..] != [0; 2]
                {
                    return Err(format!(
                        "rail replay hub row {index} has invalid fixed fields"
                    ));
                }
                hub_frames.push(RailHubFrameState {
                    province_id,
                    subtype: flags & ROW_FLAG_HUB_SUBTYPE,
                    background_button_frame: value as u8,
                    common_icon_frame: bits[0] as u8,
                    naval_overlay_visible: flags & ROW_FLAG_HUB_NAVAL_OVERLAY_VISIBLE != 0,
                });
            }
            ROW_KIND_HUB_SIMULATION => {
                hub_rows_started = true;
                if payload_version != PAYLOAD_VERSION_V3 {
                    return Err(format!(
                        "rail replay simulation row {index} is not legal before v3"
                    ));
                }
                if flags & !0x03 != 0 {
                    return Err(format!(
                        "rail replay simulation row {index} has unknown Boolean flags"
                    ));
                }
                if index + 1 >= row_count {
                    return Err(format!(
                        "rail replay simulation row {index} lacks its continuation"
                    ));
                }
                let continuation_base = base + ROW_LEN;
                if payload[continuation_base] != ROW_KIND_HUB_SIMULATION_CONTINUATION
                    || payload[continuation_base + 1..continuation_base + 4] != [0; 3]
                    || read_u32(payload, continuation_base + 4)? != read_u32(payload, base + 24)?
                    || read_u32(payload, base + 24)? as usize != hub_simulation_rows.len()
                    || read_u32(payload, base + 28)? != 0
                {
                    return Err(format!(
                        "rail replay simulation row {index} has an invalid continuation binding"
                    ));
                }
                hub_simulation_rows.push(RailHubSimulationState {
                    pov_country_id: province_id,
                    province_id: related_id,
                    province_owner_country_id: value,
                    subtype: payload[base + 2],
                    route_class: payload[base + 3],
                    node_kind: read_u32(payload, base + 16)?,
                    node_flag: payload[base + 20],
                    available_raw: read_i64(payload, continuation_base + 8)?,
                    consumed_raw: read_i64(payload, continuation_base + 16)?,
                    foreign_max_raw: read_i64(payload, continuation_base + 24)?,
                    foreign_route: flags & 1 != 0,
                    fallback_record: flags & 2 != 0,
                });
                index += 1;
            }
            ROW_KIND_HUB_SIMULATION_CONTINUATION => {
                return Err(format!(
                    "rail replay orphan simulation continuation row {index}"
                ));
            }
            _ => return Err(format!("rail replay row {index} has unknown kind {kind}")),
        }
        index += 1;
    }
    validate(
        payload_version,
        &provinces,
        &hub_frames,
        &hub_simulation_rows,
        payload_flags,
        observed_provinces,
        observed_neighbors,
    )?;
    Ok(DecodedRailReplayPayload {
        payload_version,
        elapsed_wall_ms,
        status_flags,
        payload_flags,
        observed_provinces,
        observed_neighbors,
        provinces,
        hub_frames_complete: payload_version == PAYLOAD_VERSION_V2,
        hub_frames,
        hub_simulation_complete: payload_version == PAYLOAD_VERSION_V3,
        hub_simulation_rows,
    })
}

fn validate(
    payload_version: u16,
    provinces: &[RailProvinceState],
    hub_frames: &[RailHubFrameState],
    hub_simulation_rows: &[RailHubSimulationState],
    payload_flags: u32,
    observed_provinces: u32,
    observed_neighbors: u32,
) -> Result<(), String> {
    match payload_version {
        PAYLOAD_VERSION_V1
            if payload_flags == PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE
                && hub_frames.is_empty()
                && hub_simulation_rows.is_empty() => {}
        PAYLOAD_VERSION_V2
            if payload_flags == PAYLOAD_FLAG_HUB_FRAMES_COMPLETE
                && hub_simulation_rows.is_empty() => {}
        PAYLOAD_VERSION_V3
            if payload_flags == PAYLOAD_FLAG_HUB_SIMULATION_COMPLETE && hub_frames.is_empty() => {}
        PAYLOAD_VERSION_V1 => {
            return Err("rail replay v1 must explicitly badge hub frames unavailable".into())
        }
        PAYLOAD_VERSION_V2 => {
            return Err("rail replay v2 must exclusively badge hub frames complete".into())
        }
        PAYLOAD_VERSION_V3 => {
            return Err("rail replay v3 must exclusively badge hub simulation complete".into())
        }
        _ => return Err("rail replay payload version is unsupported".into()),
    }
    if provinces.len() > MAX_PROVINCES || observed_provinces as usize != provinces.len() {
        return Err("rail replay observed province count mismatch/out of range".into());
    }
    let neighbor_count = provinces.iter().map(|p| p.neighbors.len()).sum::<usize>();
    if neighbor_count > MAX_RAIL_ROWS || observed_neighbors as usize != neighbor_count {
        return Err("rail replay observed neighbor count mismatch/out of range".into());
    }
    let mut by_id = BTreeMap::new();
    let mut previous = 0u32;
    for (index, province) in provinces.iter().enumerate() {
        if province.province_id == 0 || (index != 0 && province.province_id <= previous) {
            return Err("rail replay provinces are not strictly sorted/nonzero".into());
        }
        if province.anchor_variant > 1
            || province
                .anchor_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err(format!(
                "rail replay province {} has invalid anchor",
                province.province_id
            ));
        }
        if province.neighbors.len() > MAX_NEIGHBORS_PER_PROVINCE {
            return Err(format!(
                "rail replay province {} has too many neighbors",
                province.province_id
            ));
        }
        let mut neighbor_ids = BTreeSet::new();
        for neighbor in &province.neighbors {
            if neighbor.province_id == 0
                || neighbor.province_id == province.province_id
                || neighbor.edge_level == 0
                || neighbor.edge_level > MAX_RAIL_LEVEL
                || !neighbor_ids.insert(neighbor.province_id)
            {
                return Err(format!(
                    "rail replay province {} has invalid/duplicate neighbor",
                    province.province_id
                ));
            }
        }
        previous = province.province_id;
        by_id.insert(province.province_id, province);
    }
    for province in provinces {
        for neighbor in &province.neighbors {
            let Some(reverse) = by_id.get(&neighbor.province_id) else {
                return Err(format!(
                    "rail replay edge {}->{} has no captured endpoint",
                    province.province_id, neighbor.province_id
                ));
            };
            if !reverse
                .neighbors
                .iter()
                .any(|row| row.province_id == province.province_id)
            {
                return Err(format!(
                    "rail replay edge {}->{} lacks reciprocal row",
                    province.province_id, neighbor.province_id
                ));
            }
        }
    }
    if hub_frames.len() > MAX_HUB_FRAMES {
        return Err(format!(
            "rail replay hub frame count exceeds {MAX_HUB_FRAMES}"
        ));
    }
    let mut previous_identity = None;
    for hub in hub_frames {
        let identity = (hub.province_id, hub.subtype);
        if hub.province_id == 0
            || hub.subtype > MAX_HUB_SUBTYPE
            || !(MIN_HUB_BACKGROUND_FRAME..=MAX_HUB_BACKGROUND_FRAME)
                .contains(&hub.background_button_frame)
            || !(MIN_HUB_COMMON_ICON_FRAME..=MAX_HUB_COMMON_ICON_FRAME)
                .contains(&hub.common_icon_frame)
            || previous_identity.is_some_and(|previous| identity <= previous)
        {
            return Err(
                "rail replay hub frames are invalid or not strictly identity-sorted".into(),
            );
        }
        previous_identity = Some(identity);
    }
    if hub_simulation_rows.len() > MAX_HUB_SIMULATION_ROWS {
        return Err(format!(
            "rail replay hub simulation row count exceeds {MAX_HUB_SIMULATION_ROWS}"
        ));
    }
    let mut previous_simulation_identity = None;
    for row in hub_simulation_rows {
        let identity = (row.pov_country_id, row.province_id, row.subtype);
        if row.pov_country_id == 0
            || row.province_id == 0
            || row.province_owner_country_id == 0
            || row.subtype > MAX_HUB_SUBTYPE
            || !matches!((row.subtype, row.route_class), (0, 2) | (1, 1))
            || previous_simulation_identity.is_some_and(|previous| identity <= previous)
        {
            return Err(
                "rail replay hub simulation rows are invalid or not strictly identity-sorted"
                    .into(),
            );
        }
        previous_simulation_identity = Some(identity);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .map(|v| u16::from_le_bytes(v.try_into().expect("two bytes")))
        .ok_or_else(|| format!("rail replay u16 at {offset} truncated"))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .map(|v| u32::from_le_bytes(v.try_into().expect("four bytes")))
        .ok_or_else(|| format!("rail replay u32 at {offset} truncated"))
}
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    bytes
        .get(offset..offset + 8)
        .map(|v| u64::from_le_bytes(v.try_into().expect("eight bytes")))
        .ok_or_else(|| format!("rail replay u64 at {offset} truncated"))
}
fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, String> {
    bytes
        .get(offset..offset + 8)
        .map(|v| i64::from_le_bytes(v.try_into().expect("eight bytes")))
        .ok_or_else(|| format!("rail replay i64 at {offset} truncated"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_common_icon_selector_permutation_has_no_fallback() {
        assert_eq!(
            (0..=3)
                .map(common_icon_frame_from_selector)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(3), Some(2), Some(4)]
        );
        assert_eq!(common_icon_frame_from_selector(4), None);
    }

    #[test]
    fn round_trip_preserves_directed_levels_and_anchor_bits() {
        let rows = vec![
            RailProvinceState {
                province_id: 1,
                active_terminal: true,
                anchor_variant: 0,
                anchor_bits: [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()],
                neighbors: vec![RailNeighborState {
                    province_id: 2,
                    edge_level: 4,
                }],
            },
            RailProvinceState {
                province_id: 2,
                active_terminal: false,
                anchor_variant: 1,
                anchor_bits: [4.0f32.to_bits(), 5.0f32.to_bits(), 6.0f32.to_bits()],
                neighbors: vec![RailNeighborState {
                    province_id: 1,
                    edge_level: 2,
                }],
            },
        ];
        let bytes = encode_payload(&rows, 7, 0, PAYLOAD_FLAG_HUB_FRAMES_UNAVAILABLE, 2, 2).unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded.payload_version, PAYLOAD_VERSION_V1);
        assert!(!decoded.hub_frames_complete);
        assert!(decoded.hub_frames.is_empty());
        assert_eq!(decoded.provinces, rows);
    }

    #[test]
    fn v2_round_trip_preserves_complete_pointer_free_hub_identities() {
        let hubs = vec![
            RailHubFrameState {
                province_id: 42,
                subtype: 0,
                background_button_frame: 1,
                common_icon_frame: 3,
                naval_overlay_visible: false,
            },
            RailHubFrameState {
                province_id: 42,
                subtype: 1,
                background_button_frame: 5,
                common_icon_frame: 4,
                naval_overlay_visible: true,
            },
        ];
        let bytes = encode_payload_inner(
            PAYLOAD_VERSION_V2,
            &[],
            &hubs,
            &[],
            9,
            0,
            PAYLOAD_FLAG_HUB_FRAMES_COMPLETE,
            0,
            0,
        )
        .unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded.payload_version, PAYLOAD_VERSION_V2);
        assert!(decoded.hub_frames_complete);
        assert_eq!(decoded.hub_frames, hubs);
    }

    #[test]
    fn v3_round_trip_preserves_raw_simulation_inputs_without_viewer_state() {
        let rows = vec![
            RailHubSimulationState {
                pov_country_id: 7,
                province_id: 42,
                province_owner_country_id: 9,
                subtype: 0,
                route_class: 2,
                node_kind: 3,
                node_flag: 0,
                available_raw: 800_001,
                consumed_raw: 200_000,
                foreign_max_raw: 900_000,
                foreign_route: false,
                fallback_record: false,
            },
            RailHubSimulationState {
                pov_country_id: 7,
                province_id: 42,
                province_owner_country_id: 9,
                subtype: 1,
                route_class: 1,
                node_kind: 5,
                node_flag: 1,
                available_raw: -1,
                consumed_raw: i64::MIN,
                foreign_max_raw: i64::MAX,
                foreign_route: true,
                fallback_record: true,
            },
        ];
        let bytes = encode_payload_with_hub_simulation(&[], &rows, 11, 0, 0, 0).unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded.payload_version, PAYLOAD_VERSION_V3);
        assert!(!decoded.hub_frames_complete);
        assert!(decoded.hub_frames.is_empty());
        assert!(decoded.hub_simulation_complete);
        assert_eq!(decoded.hub_simulation_rows, rows);
        assert_eq!(read_u32(&bytes, 36).unwrap(), (rows.len() * 2) as u32);
    }

    #[test]
    fn v3_rejects_broken_pair_binding_and_duplicate_identity() {
        let row = RailHubSimulationState {
            pov_country_id: 7,
            province_id: 42,
            province_owner_country_id: 9,
            subtype: 0,
            route_class: 2,
            node_kind: 3,
            node_flag: 0,
            available_raw: 8,
            consumed_raw: 2,
            foreign_max_raw: 9,
            foreign_route: false,
            fallback_record: false,
        };
        let wire = encode_payload_with_hub_simulation(&[], &[row], 11, 0, 0, 0).unwrap();
        let mut orphan = wire.clone();
        orphan[HEADER_LEN + ROW_LEN] = 0xff;
        assert!(decode_payload(&orphan).is_err());

        let duplicate = [row, row];
        assert!(encode_payload_with_hub_simulation(&[], &duplicate, 11, 0, 0, 0).is_err());

        let mut missing_owner = row;
        missing_owner.province_owner_country_id = 0;
        assert!(encode_payload_with_hub_simulation(&[], &[missing_owner], 11, 0, 0, 0).is_err());
        let mut wrong_route_class = row;
        wrong_route_class.route_class = 1;
        assert!(
            encode_payload_with_hub_simulation(&[], &[wrong_route_class], 11, 0, 0, 0).is_err()
        );
    }
}
