//! D81 `rail_replay_state_live`: portable rail topology and supply-hub
//! simulation inputs.
//!
//! Derived from `world_fold.rs:16606`-`:16633` (`fold_rail_replay_state`).
// @world-fold-authority domain=81 functions=fold_rail_replay_state lines=16606-16633 sha256=cccace04a76fb96d48374edb442b574ccdd2beec395af042f5a4e6e8bfd26f8f closure=6b6a66d06a55a63d1cca5bb77bd2b55f4b99ebcad8def3c03a8204f979df7057
//!
//! Public schema v4 deliberately carries no GUI object, selected-country, or
//! effective-country state.  The first three collections retain the exact
//! engine-selected anchors, ordered directed adjacency, and `active_terminal`
//! byte law.  [`HUB_SIMULATION`] carries only authoritative simulation rows;
//! replay applies [`render_hub_frames`] with its own viewer-local inputs.

use crate::body::{key, Key};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;
use compact_contract::rail_replay as compact;
use portable_content_manifest::symbol_manifest::{ContentIdSource, SymbolManifestBuilder};
use std::collections::BTreeMap;

pub const DOMAIN: u32 = 81;
pub const COUNTRY_CONTENT_ID_NAMESPACE: &str = "hoi4.country";
pub const PROVINCE_CONTENT_ID_NAMESPACE: &str = "hoi4.province";

pub const FRAME: u32 = 1;
pub const PROVINCES: u32 = 2;
pub const DIRECTED_NEIGHBORS: u32 = 3;
pub const HUB_SIMULATION: u32 = 4;

pub const FRAME_INDEX: usize = 0;
pub const PROVINCES_INDEX: usize = 1;
pub const DIRECTED_NEIGHBORS_INDEX: usize = 2;
pub const HUB_SIMULATION_INDEX: usize = 3;
pub const COLLECTION_COUNT: usize = 4;
pub const SINGLETON_KEY_COMPONENT: u64 = 0;

pub mod field {
    pub const FRAME_SINGLETON: u32 = 1;
    pub const HUB_SIMULATION_COMPLETE: u32 = 2;
    pub const PROVINCE_ID: u32 = 3;
    pub const ACTIVE_TERMINAL: u32 = 4;
    pub const ANCHOR_VARIANT: u32 = 5;
    pub const ANCHOR_X: u32 = 6;
    pub const ANCHOR_Y: u32 = 7;
    pub const ANCHOR_Z: u32 = 8;
    pub const NEIGHBOR_PROVINCE_ID: u32 = 9;
    pub const EDGE_LEVEL: u32 = 10;
    pub const POV_COUNTRY_ID: u32 = 11;
    pub const HUB_SUBTYPE: u32 = 12;
    pub const PROVINCE_OWNER_COUNTRY_ID: u32 = 13;
    pub const ROUTE_CLASS: u32 = 14;
    pub const NODE_KIND: u32 = 15;
    pub const NODE_FLAG: u32 = 16;
    pub const AVAILABLE_RAW: u32 = 17;
    pub const CONSUMED_RAW: u32 = 18;
    pub const FOREIGN_MAX_RAW: u32 = 19;
    pub const FOREIGN_ROUTE: u32 = 20;
    pub const FALLBACK_RECORD: u32 = 21;
}

pub const PAYLOAD_MAGIC: u32 = compact::PAYLOAD_MAGIC;
pub const PAYLOAD_VERSION_V1: u16 = compact::PAYLOAD_VERSION_V1;
pub const PAYLOAD_VERSION_V2: u16 = compact::PAYLOAD_VERSION_V2;
pub const PAYLOAD_VERSION_V3: u16 = compact::PAYLOAD_VERSION_V3;
pub const PAYLOAD_VERSION_V4: u16 = compact::PAYLOAD_VERSION_V4;
pub const HEADER_LEN: usize = compact::HEADER_LEN;
pub const ROW_LEN: usize = compact::ROW_LEN;
pub const ROW_KIND_ANCHOR: u8 = compact::ROW_KIND_ANCHOR;
pub const ROW_KIND_NEIGHBOR: u8 = compact::ROW_KIND_NEIGHBOR;
pub const ROW_KIND_HUB_SIMULATION: u8 = compact::ROW_KIND_HUB_SIMULATION;
pub const ROW_FLAG_ACTIVE_TERMINAL: u8 = compact::ROW_FLAG_ACTIVE_TERMINAL;
pub const MAX_RAIL_LEVEL: u32 = compact::MAX_RAIL_LEVEL;
pub const ANCHOR_VARIANT_CARDINALITY: u32 = 2;
pub const EDGE_LEVEL_CARDINALITY: u32 = MAX_RAIL_LEVEL;

const NONCANONICAL_SCHEMA: &str =
    "rail replay schema does not match the canonical d81 v4 declaration";

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: FRAME,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::FRAME_SINGLETON, Kind::Unsigned).key(0),
                    Field::new(f::HUB_SIMULATION_COMPLETE, Kind::Boolean),
                ],
            },
            Collection {
                collection_id: PROVINCES,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::PROVINCE_ID, Kind::ContentId).key(0),
                    Field::new(f::ACTIVE_TERMINAL, Kind::Boolean),
                    Field::new(f::ANCHOR_VARIANT, Kind::Enum).enum_card(ANCHOR_VARIANT_CARDINALITY),
                    Field::new(f::ANCHOR_X, Kind::Continuous).quant(Quant::Exact),
                    Field::new(f::ANCHOR_Y, Kind::Continuous).quant(Quant::Exact),
                    Field::new(f::ANCHOR_Z, Kind::Continuous).quant(Quant::Exact),
                ],
            },
            Collection {
                collection_id: DIRECTED_NEIGHBORS,
                shape: Shape::OrderedKeyedList,
                fields: vec![
                    Field::new(f::PROVINCE_ID, Kind::ContentId).key(0),
                    Field::new(f::NEIGHBOR_PROVINCE_ID, Kind::ContentId).key(1),
                    Field::new(f::EDGE_LEVEL, Kind::Enum).enum_card(EDGE_LEVEL_CARDINALITY),
                ],
            },
            Collection {
                collection_id: HUB_SIMULATION,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(f::POV_COUNTRY_ID, Kind::ContentId).key(0),
                    Field::new(f::PROVINCE_ID, Kind::ContentId).key(1),
                    Field::new(f::HUB_SUBTYPE, Kind::Unsigned).key(2),
                    Field::new(f::PROVINCE_OWNER_COUNTRY_ID, Kind::ContentId),
                    Field::new(f::ROUTE_CLASS, Kind::Unsigned),
                    Field::new(f::NODE_KIND, Kind::Unsigned),
                    Field::new(f::NODE_FLAG, Kind::Unsigned),
                    Field::new(f::AVAILABLE_RAW, Kind::Signed),
                    Field::new(f::CONSUMED_RAW, Kind::Signed),
                    Field::new(f::FOREIGN_MAX_RAW, Kind::Signed),
                    Field::new(f::FOREIGN_ROUTE, Kind::Boolean),
                    Field::new(f::FALLBACK_RECORD, Kind::Boolean),
                ],
            },
        ],
        event_part: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub payload_version: u16,
    pub status_flags: u32,
    pub elapsed_wall_ms: u64,
    pub observed_provinces: u32,
    pub observed_neighbors: u32,
    pub hub_simulation_complete: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedPayload {
    pub header: PayloadHeader,
    pub frame: (Key, Record),
    pub provinces: Vec<(Key, Record)>,
    /// Producer order is rendering-visible and is retained exactly.
    pub directed_neighbors: Vec<(Key, Record)>,
    pub hub_simulation_rows: Vec<(Key, Record)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FoldOutcome {
    Snapshot(DecodedPayload),
    Unavailable { status_flags: u32 },
}

/// Parse the complete compact carrier. The returned byte region is retained
/// for the existing wire-product audit seam; v4 mapping uses the typed compact
/// decoder and never reparses UI-era row layouts.
pub fn split_payload(payload: &[u8]) -> Result<(PayloadHeader, &[u8]), String> {
    let decoded = compact::decode_payload(payload)
        .map_err(|error| format!("rail_replay_state RRL1 invalid: {error}"))?;
    Ok((
        PayloadHeader {
            payload_version: decoded.payload_version,
            status_flags: decoded.status_flags,
            elapsed_wall_ms: decoded.elapsed_wall_ms,
            observed_provinces: decoded.observed_provinces,
            observed_neighbors: decoded.observed_neighbors,
            hub_simulation_complete: decoded.hub_simulation_complete,
        },
        &payload[HEADER_LEN..],
    ))
}

pub fn fold_payload(
    supplied_schema: &Schema,
    payload: &[u8],
    frame_tick: u64,
    symbols: &SymbolManifestBuilder,
) -> Result<FoldOutcome, String> {
    require_canonical_schema(supplied_schema)?;
    let decoded = compact::decode_payload(payload)
        .map_err(|error| format!("rail_replay_state RRL1 invalid: {error}"))?;
    let unknown = compact_contract::compact_status_unknown_flags_v1(decoded.status_flags);
    if unknown != 0 {
        return Err(format!(
            "rail replay status_flags has unknown global bits {unknown:#x}"
        ));
    }
    if compact_contract::compact_status_is_failure(decoded.status_flags) {
        return Ok(FoldOutcome::Unavailable {
            status_flags: decoded.status_flags,
        });
    }
    if decoded.payload_version != PAYLOAD_VERSION_V4 {
        return Err(format!(
            "rail replay payload v{} is historical UI-era/non-production; d81 public schema requires v4",
            decoded.payload_version
        ));
    }
    if !decoded.hub_simulation_complete {
        return Err("rail replay v4 does not badge a complete simulation snapshot".into());
    }

    validate_symbol_injectivity(&decoded, symbols)?;
    let frame = (
        key(&[SINGLETON_KEY_COMPONENT]),
        Record::new(vec![Value::Bool(true)]),
    );
    let provinces = decoded
        .provinces
        .iter()
        .map(|province| {
            Ok((
                key(&[resolve_province(symbols, province.province_id)?]),
                Record::new(vec![
                    Value::Bool(province.active_terminal),
                    Value::Enum(u32::from(province.anchor_variant)),
                    Value::Continuous(f64::from(f32::from_bits(province.anchor_bits[0]))),
                    Value::Continuous(f64::from(f32::from_bits(province.anchor_bits[1]))),
                    Value::Continuous(f64::from(f32::from_bits(province.anchor_bits[2]))),
                ]),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut directed_neighbors = Vec::with_capacity(decoded.observed_neighbors as usize);
    for province in &decoded.provinces {
        for neighbor in &province.neighbors {
            directed_neighbors.push((
                key(&[
                    resolve_province(symbols, province.province_id)?,
                    resolve_province(symbols, neighbor.province_id)?,
                ]),
                Record::new(vec![Value::Enum(neighbor.edge_level - 1)]),
            ));
        }
    }
    let hub_simulation_rows = decoded
        .hub_simulation_rows
        .iter()
        .map(|row| map_simulation_row(row, symbols))
        .collect::<Result<Vec<_>, String>>()?;

    Ok(FoldOutcome::Snapshot(DecodedPayload {
        header: PayloadHeader {
            payload_version: decoded.payload_version,
            status_flags: decoded.status_flags,
            elapsed_wall_ms: decoded.elapsed_wall_ms,
            observed_provinces: decoded.observed_provinces,
            observed_neighbors: decoded.observed_neighbors,
            hub_simulation_complete: decoded.hub_simulation_complete,
        },
        frame,
        provinces,
        directed_neighbors,
        hub_simulation_rows,
    }))
}

fn map_simulation_row(
    row: &compact::RailHubSimulationState,
    symbols: &SymbolManifestBuilder,
) -> Result<(Key, Record), String> {
    if row.province_owner_country_id == 0
        || !matches!((row.subtype, row.route_class), (0, 2) | (1, 1))
    {
        return Err(
            "rail replay v4 simulation row has invalid owner or subtype/route-class join".into(),
        );
    }
    Ok((
        key(&[
            resolve_country(symbols, row.pov_country_id)?,
            resolve_province(symbols, row.province_id)?,
            u64::from(row.subtype),
        ]),
        Record::new(vec![
            Value::ContentId(resolve_country(symbols, row.province_owner_country_id)?),
            Value::Unsigned(u64::from(row.route_class)),
            Value::Unsigned(u64::from(row.node_kind)),
            Value::Unsigned(u64::from(row.node_flag)),
            Value::Signed(row.available_raw),
            Value::Signed(row.consumed_raw),
            Value::Signed(row.foreign_max_raw),
            Value::Bool(row.foreign_route),
            Value::Bool(row.fallback_record),
        ]),
    ))
}

fn resolve_country(symbols: &SymbolManifestBuilder, source_country_id: u32) -> Result<u64, String> {
    resolve_content(
        symbols,
        COUNTRY_CONTENT_ID_NAMESPACE,
        source_country_id,
        "country",
    )
}

fn resolve_province(
    symbols: &SymbolManifestBuilder,
    source_province_id: u32,
) -> Result<u64, String> {
    resolve_content(
        symbols,
        PROVINCE_CONTENT_ID_NAMESPACE,
        source_province_id,
        "province",
    )
}

fn resolve_content(
    symbols: &SymbolManifestBuilder,
    namespace: &str,
    source_id: u32,
    label: &str,
) -> Result<u64, String> {
    symbols
        .require_content_id(&ContentIdSource::new(namespace, u64::from(source_id)))
        .map(|id| id.get())
        .map_err(|error| format!("rail replay {label} identity is not portable: {error}"))
}

fn validate_symbol_injectivity(
    decoded: &compact::DecodedRailReplayPayload,
    symbols: &SymbolManifestBuilder,
) -> Result<(), String> {
    let mut provinces = BTreeMap::<u64, u32>::new();
    let mut countries = BTreeMap::<u64, u32>::new();
    for province in &decoded.provinces {
        insert_injective(
            &mut provinces,
            resolve_province(symbols, province.province_id)?,
            province.province_id,
            "province",
        )?;
        for neighbor in &province.neighbors {
            insert_injective(
                &mut provinces,
                resolve_province(symbols, neighbor.province_id)?,
                neighbor.province_id,
                "province",
            )?;
        }
    }
    for row in &decoded.hub_simulation_rows {
        insert_injective(
            &mut provinces,
            resolve_province(symbols, row.province_id)?,
            row.province_id,
            "province",
        )?;
        for country_id in [row.pov_country_id, row.province_owner_country_id] {
            insert_injective(
                &mut countries,
                resolve_country(symbols, country_id)?,
                country_id,
                "country",
            )?;
        }
    }
    Ok(())
}

fn insert_injective(
    by_symbol: &mut BTreeMap<u64, u32>,
    symbol: u64,
    raw: u32,
    label: &str,
) -> Result<(), String> {
    if let Some(previous) = by_symbol.insert(symbol, raw) {
        if previous != raw {
            return Err(format!(
                "rail replay {label} identity is not injective: raw ids {previous} and {raw} resolve to portable symbol id {symbol}"
            ));
        }
    }
    Ok(())
}

fn require_canonical_schema(supplied: &Schema) -> Result<(), String> {
    if supplied != &schema() {
        return Err(NONCANONICAL_SCHEMA.into());
    }
    Ok(())
}

/// Replay-only inputs. These values are never part of a D81 payload or public
/// collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HubViewerInputs {
    pub selected_province_id: Option<u32>,
    /// Canonical D18 country identity after the authenticated
    /// `CGameState+0x340` equivalence collapse.
    pub effective_country_id: u32,
}

/// Deterministic GUI outputs mechanically derived at replay time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HubRenderedFrames {
    pub background_button_frame: u8,
    pub common_icon_frame: u8,
    pub naval_overlay_visible: bool,
}

/// Apply the exact a729 supply-hub visual law to one simulation row.
///
/// `selected` means the viewer selected this row's province;
/// `same_owner` compares canonical country identities. The collector collapses
/// not raw engine-handle equality.
pub fn render_hub_frames(
    row: &compact::RailHubSimulationState,
    viewer: HubViewerInputs,
) -> HubRenderedFrames {
    let selected = viewer.selected_province_id == Some(row.province_id);
    let same_owner = viewer.effective_country_id == row.province_owner_country_id;

    let background_selector = if row.node_flag != 0 {
        3 + u8::from(selected)
    } else if selected {
        2
    } else if same_owner {
        1
    } else {
        0
    };

    let delta = i128::from(row.available_raw) - i128::from(row.consumed_raw);
    let scale = 100_000i128;
    let ceil_delta = delta.div_euclid(scale) + i128::from(delta.rem_euclid(scale) != 0);
    let zero_delta = ceil_delta == 0 && !row.foreign_route;
    let common_selector = if !zero_delta || row.node_flag != 0 {
        2
    } else if row.foreign_max_raw > row.available_raw || selected {
        3
    } else if !same_owner {
        1
    } else {
        0
    };
    let common_icon_frame = match common_selector {
        0 => 1,
        1 => 3,
        2 => 2,
        3 => 4,
        _ => unreachable!("exact selector law yields 0..=3"),
    };

    HubRenderedFrames {
        background_button_frame: background_selector + 1,
        common_icon_frame,
        naval_overlay_visible: matches!(row.node_kind, 3 | 5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TickGrid;
    use portable_content_manifest::symbol_manifest::SymbolObservation;

    fn sim() -> compact::RailHubSimulationState {
        compact::RailHubSimulationState {
            pov_country_id: 1,
            province_id: 10,
            province_owner_country_id: 2,
            subtype: 1,
            route_class: 1,
            node_kind: 5,
            node_flag: 0,
            available_raw: 100_000,
            consumed_raw: 100_000,
            foreign_max_raw: 90_000,
            foreign_route: false,
            fallback_record: false,
        }
    }

    #[test]
    fn schema_v4_is_canonical_and_has_no_ui_fields() {
        let schema = schema();
        schema.validate(TickGrid::HOURLY.log2()).unwrap();
        assert_eq!(schema.collections.len(), COLLECTION_COUNT);
        assert_eq!(
            schema.collections[DIRECTED_NEIGHBORS_INDEX].shape,
            Shape::OrderedKeyedList
        );
        assert_eq!(
            schema.collections[HUB_SIMULATION_INDEX].shape,
            Shape::KeyedSet
        );
        let simulation = &schema.collections[HUB_SIMULATION_INDEX];
        assert_eq!(
            simulation
                .fields
                .iter()
                .map(|field| field.field_id)
                .collect::<Vec<_>>(),
            vec![
                field::POV_COUNTRY_ID,
                field::PROVINCE_ID,
                field::HUB_SUBTYPE,
                field::PROVINCE_OWNER_COUNTRY_ID,
                field::ROUTE_CLASS,
                field::NODE_KIND,
                field::NODE_FLAG,
                field::AVAILABLE_RAW,
                field::CONSUMED_RAW,
                field::FOREIGN_MAX_RAW,
                field::FOREIGN_ROUTE,
                field::FALLBACK_RECORD,
            ],
            "public schema is raw simulation input only"
        );
    }

    #[test]
    fn rendering_law_covers_owner_selection_delta_and_naval_cases() {
        let row = sim();
        assert_eq!(
            render_hub_frames(
                &row,
                HubViewerInputs {
                    selected_province_id: None,
                    effective_country_id: 2,
                }
            ),
            HubRenderedFrames {
                background_button_frame: 2,
                common_icon_frame: 1,
                naval_overlay_visible: true,
            }
        );
        assert_eq!(
            render_hub_frames(
                &row,
                HubViewerInputs {
                    selected_province_id: Some(10),
                    effective_country_id: 2,
                }
            ),
            HubRenderedFrames {
                background_button_frame: 3,
                common_icon_frame: 4,
                naval_overlay_visible: true,
            }
        );
        let mut flagged = row;
        flagged.node_flag = 1;
        assert_eq!(
            render_hub_frames(
                &flagged,
                HubViewerInputs {
                    selected_province_id: Some(10),
                    effective_country_id: 2,
                }
            ),
            HubRenderedFrames {
                background_button_frame: 5,
                common_icon_frame: 2,
                naval_overlay_visible: true,
            }
        );
        let mut nonzero = row;
        nonzero.available_raw = i64::MAX;
        nonzero.consumed_raw = i64::MIN;
        nonzero.node_kind = 4;
        assert_eq!(
            render_hub_frames(
                &nonzero,
                HubViewerInputs {
                    selected_province_id: None,
                    effective_country_id: 99,
                }
            ),
            HubRenderedFrames {
                background_button_frame: 1,
                common_icon_frame: 2,
                naval_overlay_visible: false,
            }
        );
    }

    #[test]
    fn viewer_inputs_do_not_change_simulation_record_or_key() {
        let mut symbols = SymbolManifestBuilder::from_base([
            "country/tag/GER",
            "country/tag/FRA",
            "province/rgb/1/2/3",
        ])
        .unwrap();
        symbols
            .bind_existing_sources([
                SymbolObservation::content_id(COUNTRY_CONTENT_ID_NAMESPACE, 1, "country/tag/GER"),
                SymbolObservation::content_id(COUNTRY_CONTENT_ID_NAMESPACE, 2, "country/tag/FRA"),
                SymbolObservation::content_id(
                    PROVINCE_CONTENT_ID_NAMESPACE,
                    10,
                    "province/rgb/1/2/3",
                ),
            ])
            .unwrap();
        let row = sim();
        let mapped = map_simulation_row(&row, &symbols).unwrap();
        let mut before = Vec::new();
        mapped
            .1
            .encode(&schema().collections[HUB_SIMULATION_INDEX], &mut before)
            .unwrap();
        let _ = render_hub_frames(
            &row,
            HubViewerInputs {
                selected_province_id: Some(10),
                effective_country_id: 2,
            },
        );
        let mut after = Vec::new();
        mapped
            .1
            .encode(&schema().collections[HUB_SIMULATION_INDEX], &mut after)
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            mapped.1.values,
            vec![
                Value::ContentId(resolve_country(&symbols, 2).unwrap()),
                Value::Unsigned(1),
                Value::Unsigned(5),
                Value::Unsigned(0),
                Value::Signed(100_000),
                Value::Signed(100_000),
                Value::Signed(90_000),
                Value::Bool(false),
                Value::Bool(false),
            ]
        );
    }
}
