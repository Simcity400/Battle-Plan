pub use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub const SCHEMA: &str = "hoi4.production-render-submit.v17";
pub const INCOMPATIBLE_PRODUCTION_FRAME_CENSUS_SCHEMA: &str = "hoi4.production-render-submit.v16";
pub const INCOMPATIBLE_TILED_LIGHT_PAYLOAD_SCHEMA: &str = "hoi4.production-render-submit.v15";
pub const INCOMPATIBLE_DERIVED_RUNTIME_CARRIER_SCHEMA: &str = "hoi4.production-render-submit.v14";
pub const INCOMPATIBLE_PRESENTATION_CARRIER_SCHEMA: &str = "hoi4.production-render-submit.v13";
pub const INCOMPATIBLE_AUTHENTICATED_TEXTURE_SNAPSHOT_SCHEMA: &str =
    "hoi4.production-render-submit.v12";
pub const INCOMPATIBLE_RECORDED_INPUT_OBSERVATION_SCHEMA: &str =
    "hoi4.production-render-submit.v11";
pub const RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256: &str =
    "39b71a6fb75e53e31136e1909452b85106baf83addeb78da92901113cd98f0b0";
pub const INCOMPATIBLE_SNOW_MUD_FADE_SCHEMA: &str = "hoi4.production-render-submit.v10";
pub const MAPNAME_TRANSP_OFFSET_SEMANTIC: &str =
    "gfx/FX/mapname.shader:ConstantBuffer(1,32):Transp_OffsetX";
pub const INCOMPATIBLE_WEATHER_CARRIER_SCHEMA: &str = "hoi4.production-render-submit.v9";
pub const INCOMPATIBLE_SNOW_MUD_PRESENT_LINK_SCHEMA: &str = "hoi4.production-render-submit.v8";
pub const INCOMPATIBLE_CAMERA_EVENT_SCHEMA: &str = "hoi4.production-render-submit.v7";
pub const INCOMPATIBLE_S1_SHADOW_SCHEMA: &str = "hoi4.production-render-submit.v6";
pub const INCOMPATIBLE_TREE_RUNTIME_SCHEMA: &str = "hoi4.production-render-submit.v5";
pub const INCOMPATIBLE_SNOW_MUD_CARRIER_SCHEMA: &str = "hoi4.production-render-submit.v4";
pub const INCOMPATIBLE_CAMERA_CARRIER_SCHEMA: &str = "hoi4.production-render-submit.v3";
pub const INCOMPATIBLE_ENTITY_SELECTOR_SCHEMA: &str = "hoi4.production-render-submit.v2";
pub const A729_CAMERA_TEMPORAL_SCHEMA: &str = "hoi4.camera-temporal.a729.v1";
pub const A729_CAMERA_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const A729_CAMERA_ENVELOPE_RVA: u32 = 0x0222_8df0;

/// The A729 camera carrier slot table: ordinal -> carrier pointer offset from
/// the canonical C00 root, as observed on hoi4.exe 1.19.2.0.a729 (the envelope
/// hook receives a per-slot carrier pointer whose root is carrier - offset).
/// The sealed carrier's own `carrier_offset` must name its slot here, which is
/// how a viewer re-derives the ordinal without a capture-side scalar.
pub const A729_CAMERA_CARRIER_OFFSETS: [u32; 3] = [0x10, 0x230, 0x450];
pub const A729_CAMERA_UPDATE_RVA: u32 = 0x0124_e290;
pub const A729_CAMERA_VERTICAL_EVENT_PRODUCER_RVA: u32 = 0x00db_a7c0;
pub const A729_CAMERA_MAP_PICK_RVA: u32 = 0x00b4_4760;
pub const A729_CAMERA_RENDER_VIEW_BYTES: usize = 0x1b4;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const SNOW_MUD_TEXTURE_SCHEMA: &str = "hoi4.a729.snow-mud-texture.v3";
pub const SNOW_MUD_H2_READBACK_SCHEMA: &str = "hoi4.replay.snow-mud-h2-readback.v3";
pub const SNOW_MUD_TEXTURE_SEMANTIC: &str = "SnowMudTexture";
pub const SNOW_MUD_FADE_VARIABLE: &str = "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed";
pub const SNOW_MUD_FADE_COMPONENT_BYTE_OFFSET: u32 = 8;
pub const SNOW_MUD_PRODUCER_FUNCTION_RVA: u32 = 0x0124_B710;
pub const SNOW_MUD_UPLOAD_CALLSITE_RVA: u32 = 0x0124_BAA4;
pub const SNOW_MUD_UPLOAD_TARGET_RVA: u32 = 0x0240_0E40;
pub const SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256: &str =
    "67685a554535ca8924a63558347287cf1e60c70b493f878bddb01c2ffe2e995c";
pub const SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256: &str =
    "9915fc3252c0e4f5a10e7b1814dd01e89beb7ca555c3162dc21305647e81476e";
pub const SNOW_MUD_TEXTURE_WIDTH: u32 = 1_408;
pub const SNOW_MUD_TEXTURE_HEIGHT: u32 = 512;
pub const SNOW_MUD_TEXTURE_BYTES_PER_TEXEL: u32 = 4;
pub const SNOW_MUD_TEXTURE_ROW_PITCH: u32 =
    SNOW_MUD_TEXTURE_WIDTH * SNOW_MUD_TEXTURE_BYTES_PER_TEXEL;
pub const SNOW_MUD_TEXTURE_BYTE_LENGTH: usize =
    SNOW_MUD_TEXTURE_ROW_PITCH as usize * SNOW_MUD_TEXTURE_HEIGHT as usize;
fn compact_update_index_from_raw(raw: i32) -> Option<u64> {
    portable_admission::hour_index_from_raw(raw).and_then(|hour| u64::try_from(hour).ok())
}

mod foundation;
pub use foundation::{
    collapse_element_diagnostics, ElementDiagnostic, ElementOutcome, FamilyCoverageRow,
    FamilyDisposition, FoundationGap, FoundationOperationIdentity, FoundationRenderEnvelope,
    MapVisualFamily, FOUNDATION_RENDER_SCHEMA, MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY,
};

mod xmap07_snow_mud;
pub use xmap07_snow_mud::{
    grid_from_tiles, tiles_from_grid, Xmap07PassiveSnowMudCarrier, Xmap07TileLayout,
    XMAP07_PASSIVE_SNOW_MUD_KEY, XMAP07_PASSIVE_SNOW_MUD_SCHEMA, XMAP07_PASSIVE_SNOW_MUD_SCOPE,
    XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256, XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
    XMAP07_SNOW_MUD_FORMAT, XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256,
    XMAP07_SNOW_MUD_SCENE_COPY_RVA, XMAP07_SNOW_MUD_TILE_EDGE,
};

/// The EIR-CW-XMAP14 combat-indicator carrier the recorder writes and the
/// XMAP14 adapter reads. Public as a MODULE, not flattened: eleven of its names
/// are generic enough (`LandCarrier`, `MissingOperand`) that a re-export would
/// read as this crate's own vocabulary rather than XMAP14's.
pub mod xmap14_combat_indicators;

/// The three-state leaf the replay reads every typed view through, with the
/// wire form the shared records carry it in.
mod leaf;
pub use leaf::Leaf;

/// The combat records (`combats[]`, `ended_combats[]`, `naval_combats`) the
/// recorder writes and the replay reads, as ONE struct each. A module, like
/// XMAP14's, because its names (`LandCombatDetail`, `NavalCombatMemberRow`) are
/// the replay's vocabulary rather than this crate's.
pub mod combat_records;

mod assigned_stage;
pub use assigned_stage::{
    AssignedStage, AssignedStageEdge, AssignedStageIdentity, AssignedStageSchedule,
    AssignedStageTopology, ASSIGNED_STAGE_SCHEDULE_SCHEMA, ASSIGNED_STAGE_SCHEMA,
    ASSIGNED_STAGE_TOPOLOGY_SCHEMA,
};

mod xmap20;
pub use xmap20::{
    Xmap20DependencyReceipt, Xmap20DependencyReceipts, XMAP20_CAMERA_OWNER,
    XMAP20_ENTITY_IDENTITY_OWNER, XMAP20_LIGHTING_OWNER, XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER,
    XMAP20_PROVIDER_RECEIPTS_SCHEMA, XMAP20_WEATHER_IDENTITY_OWNER,
};

mod xmap20_effects;
pub use xmap20_effects::{
    admit_xmap20_effects, validate_xmap20_admitted_particles, Xmap20ActiveEffect,
    Xmap20EffectAdmission, Xmap20EffectFrame, Xmap20LifecycleEvent, Xmap20ParticleSubmissionRef,
    XMAP20_ACTIVE_FLAGS_ASSIGNED_MASK, XMAP20_ACTIVE_FLAG_ENABLED,
    XMAP20_PRODUCER_KIND_UNATTRIBUTED, XMAP20_PRODUCER_KIND_UNIT, XMAP20_PRODUCER_KIND_WEATHER,
    XMAP20_EFFECT_ADMISSION_SCHEMA, XMAP20_EFFECT_FRAME_SCHEMA, XMAP20_EXECUTABLE_SHA256,
    XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA, XMAP20_PARTICLE_REGISTRY_RVA,
    XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA,
};

mod carrier_producer_contract;
pub use carrier_producer_contract::{
    carrier_leaf_source_text, validate_carrier_contract_table, CarrierGateOwner, CarrierHourGate,
    CarrierLeafSource,
};

mod xmap13_producer_contract;
pub use xmap13_producer_contract::{
    validate_xmap13_producer_contract,
    xmap13_classify_envelope_leaf, xmap13_classify_row_leaf,
    xmap13_canonical_air_wing_id, xmap13_canonical_division_id, xmap13_canonical_task_force_id,
    xmap13_wing_anchor_province, Xmap13AirBaseAnchor, Xmap13WingAnchorRefusal,
    XMAP13_SHIP_HANDLE_TYPE,
    XMAP13_MANAGER_ARMY_ROW_FIELDS, XMAP13_MANAGER_FIELDS_V1, XMAP13_MANAGER_FIELDS_V2,
    XMAP13_MANAGER_MAX_ARMY_ID, XMAP13_MANAGER_MAX_COHESION_TYPE,
    XMAP13_MANAGER_MAX_LEADER_PROXIMITY, XMAP13_MANAGER_MAX_REGISTERED_ENTRY_COUNT,
    XMAP13_ORDERS_GROUP_MANAGER_AUTHORITY,
    xmap13_classify_task_force_member_leaf, xmap13_contract_gate_leaves, xmap13_minted_row_leaves,
    xmap13_row_contract_gate_leaves,
        Xmap13ProducerGate, XMAP13_CARRIER_WIRE_KEY, XMAP13_ENTRIES_WITH_A_MEMBER_TABLE,
    XMAP13_ENVELOPE_CONTRACT, XMAP13_PRODUCER_CONTRACT_SCHEMA, XMAP13_PRODUCER_GATE_SCHEMA,
    XMAP13_IDENTITY_KIND_AIR_WING, XMAP13_IDENTITY_KIND_DIVISION, XMAP13_IDENTITY_KIND_TASK_FORCE,
    XMAP13_PRODUCER_GATE_WIRE_KEY, XMAP13_RECORDED_INPUTS, XMAP13_ROW_CONTRACT,
    XMAP13_TASK_FORCE_MEMBER_CONTRACT,
};

mod xmap14_producer_contract;
pub use xmap14_producer_contract::{
    validate_xmap14_producer_contract, xmap14_classify_envelope_leaf, xmap14_classify_land_leaf,
    xmap14_classify_naval_leaf, xmap14_classify_naval_result_leaf, xmap14_classify_unplaced_leaf,
    xmap14_contract_gate_leaves, xmap14_viewer_derived_leaves, XMAP14_CARRIER_WIRE_PATH,
    XMAP14_ENVELOPE_CONTRACT, XMAP14_LAND_ANCHOR_LEAVES, XMAP14_LAND_ATTACKER_FAMILY_BASE_KIND,
    XMAP14_LAND_ATTACKER_FAMILY_LEN, XMAP14_LAND_BASE_LOCATOR_KIND, XMAP14_LAND_ROW_CONTRACT,
    XMAP14_NAVAL_RESULT_ROW_CONTRACT, XMAP14_NAVAL_ROW_CONTRACT,
    XMAP14_PRODUCER_CONTRACT_SCHEMA, XMAP14_RECORDED_INPUTS, XMAP14_TABLES,
    XMAP14_UNPLACED_ROW_CONTRACT,
};

mod xmap15_producer_contract;
pub use xmap15_producer_contract::{
    validate_xmap15_producer_contract, xmap15_classify_envelope_leaf,
    xmap15_classify_movement_leaf, xmap15_classify_plan_leaf, xmap15_contract_gate_leaves,
    xmap15_minted_plan_leaves, Xmap15HourGate, Xmap15ProducerGate, XMAP15_BLOCKING_DOMAIN_ID,
    XMAP15_CARRIER_WIRE_PATH, XMAP15_D54_MAX_ARMY_ID, XMAP15_D54_MAX_MAP_ID,
    D54OrderLinkIdentity, D54FrontBinding, D54OrderPreparationState, D54OrderedOrderLinks, parse_d54_order_links, D54OwnerColourRow, D54OwnerColourCensus,
    XMAP15_D54_MAX_ORDER_INSTANCE_ID, XMAP15_D54_ORDER_ROW_FIELDS, XMAP15_D54_SLOT_INDEX_COUNT,
    XMAP15_D54_STYLE_FALLBACK, XMAP15_D54_STYLE_FLAG_MASK, XMAP15_D54_STYLE_MOVE_SPECIAL,
    XMAP15_D54_STYLE_OWNER_GROUPED, XMAP15_D54_STYLE_OWNER_OVERRIDE, XMAP15_D54_STYLE_VIRTUAL,
    XMAP15_D54_STYLE_WITHDRAW,
    XMAP15_ENVELOPE_CONTRACT, XMAP15_MOVEMENT_ROW_CONTRACT,
    XMAP15_PLAN_ROW_CONTRACT, XMAP15_PRODUCER_CONTRACT_SCHEMA, XMAP15_PRODUCER_GATE_SCHEMA,
    XMAP15_PRODUCER_GATE_WIRE_PATH, XMAP15_RECORDED_INPUTS, XMAP15_SUBMITTED_MESH_LEAVES,
};

mod c07_trees;
pub use c07_trees::{
    C07TreeBucketCarrier, C07TreeCellOrdinalRange, C07TreeContentLibrary, C07TreeContentSelection,
    C07TreeCoverage, C07TreeCoverageKind, C07TreeCoverageRequirement, C07TreeDrawRef,
    C07TreeImmutableBucketContent, C07TreePassCarrier, C07TreePassKind, C07TreeRuntimeCarrier,
    C07_TREE_A729_EXECUTABLE_SHA256, C07_TREE_BUFFER_FACTORY_CALLSITE_RVA,
    C07_TREE_CONTENT_LIBRARY_BUCKETS, C07_TREE_CONTENT_LIBRARY_CELLS,
    C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256, C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256,
    C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256, C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES,
    C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256, C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256,
    C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256, C07_TREE_CONTENT_LIBRARY_SCHEMA,
    C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS, C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS,
    C07_TREE_CONTENT_SELECTION_SCHEMA, C07_TREE_GLOBAL_NONZERO_CELLS,
    C07_TREE_INSTANCE_STRIDE_BYTES, C07_TREE_PRODUCER_FUNCTION_RVA, C07_TREE_RETAINED_BUCKETS,
    C07_TREE_RETAINED_CONTRACT_SHA256, C07_TREE_RETAINED_PASSES, C07_TREE_RETAINED_RDC_SHA256,
    C07_TREE_RETAINED_VISIBLE_CELLS, C07_TREE_RUNTIME_SCHEMA, C07_TREE_UNCAPTURED_CELLS,
};

mod c07_object_contract;
pub use c07_object_contract::{
    MapObjectDrawArguments, MapObjectMaterialFamily, MapObjectMeshGroupContent,
    MapObjectRenderContent, MapObjectSubmeshContent, PreparedMapObjectDraw,
    MAP_OBJECT_PARTITION_ALL, MAP_OBJECT_PARTITION_PRIMARY, MAP_OBJECT_PARTITION_WRAP_EAST,
    MAP_OBJECT_PARTITION_WRAP_WEST,
};

mod s1_shadow;
pub use s1_shadow::{
    S1DrawArguments, S1LightDepthSubmission, S1SelectedLight, S1ShadowCarrier, S1ShadowPipeline,
    S1TargetBinding, S1TypedOwner, S1_A729_EXECUTABLE_SHA256, S1_CHAIN_AUTHORITY_SHA256,
    S1_L02_AUTHORITY_SHA256, S1_L03_AUTHORITY_SHA256, S1_LIGHT_DEPTH_DXGI_FORMAT,
    S1_LIGHT_DEPTH_HEIGHT, S1_LIGHT_DEPTH_WIDTH, S1_PARTITION_ALL, S1_PARTITION_PRIMARY,
    S1_PARTITION_WRAP_EAST, S1_PARTITION_WRAP_WEST, S1_SCREEN_DXGI_FORMAT, S1_SHADOW_SCHEMA,
    S1_TARGET_BIND_FLAGS, S1_TARGET_MIP_LEVELS,
};

pub mod exact_math;
mod xmap15_paradrop;
pub use xmap15_paradrop::Xmap15ParadropProfile;

mod m03_arrows;
pub use m03_arrows::{
    Xmap15ViewerRuntimeFrame, Xmap15ViewerPlanDrawInputs, Xmap15ViewerMissingInput, Xmap15ViewerPlanRefusal,
    xmap15_semantic_producer_registration_sha256, M03ArrowDraw, M03ArrowDrawMember,
    M03ArrowDrawMemberRef, M03ArrowDrawRef, M03ArrowFrame, M03ArrowRow, M03PlanSubtype,
    M03StableKey, M03Visibility, Xmap15MapArrowVertex, Xmap15MovementDefinition, Xmap15MovementKey,
    Xmap15MovementRoute, Xmap15MovementSemanticRow, Xmap15OwnerKind, Xmap15PlanDefinitionSelector,
    Xmap15PlanGeometryProfile, Xmap15PlanKey, Xmap15PlanMapArrowMesh, Xmap15PlanSemanticRow,
    Xmap15PreparedFrame, Xmap15PreparedRoute, Xmap15PreparedRow, Xmap15SemanticFrame,
    M03_A729_EXECUTABLE_SHA256, M03_ARROW_SCHEMA, M03_STATIC_PROOF_SHA256,
    XMAP15_SEMANTIC_INPUT_SCHEMA, XMAP15_SEMANTIC_PRODUCER_REGISTRATION_DESCRIPTOR,
    XMAP15_STATE_FIRST_SCHEMA,
};

mod rail_supply_arrows;
pub use rail_supply_arrows::{
    RailSupplyArrowDefinition, RailSupplyArrowDraw, RailSupplyArrowDrawMember,
    RailSupplyArrowDrawMemberRef, RailSupplyArrowDrawPass, RailSupplyArrowDrawRef,
    RailSupplyArrowFrame, RailSupplyArrowGeometryProfile, RailSupplyArrowKey, RailSupplyArrowRow,
    RailSupplyColorRole, RailSupplyConstructionMode, RailSupplyHighlightOverride,
    RailSupplyMapArrowMesh, RailSupplyMapArrowMeshDraw, RailSupplyMapArrowVertex,
    RailSupplyVisibility, RAIL_SUPPLY_A729_EXECUTABLE_SHA256, RAIL_SUPPLY_ARROW_SCHEMA,
    RAIL_SUPPLY_GRAPHICS_DEFINES_FILE_SHA256, RAIL_SUPPLY_GRAPHICS_DEFINES_PATH,
    RAIL_SUPPLY_MAPARROWS_FILE_SHA256, RAIL_SUPPLY_MAPARROWS_PATH,
    RAIL_SUPPLY_OWNER_MANIFEST_SHA256, RAIL_SUPPLY_PRODUCER_MANIFEST_SHA256,
    RAIL_SUPPLY_PROVINCES_BMP_PATH, RAIL_SUPPLY_SELECTOR_MANIFEST_SHA256,
};

mod o04_cities;
pub use o04_cities::{
    O04CityFrame, O04CitySemantic, O04CitylightState, O04_A729_EXECUTABLE_SHA256, O04_CITY_SCHEMA,
    O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256,
};
mod o05_buildings;
pub use o05_buildings::{
    O05BuildingFrame, O05BuildingObjectKind, O05BuildingRenderDisposition, O05BuildingSemantic,
    O05_A729_EXECUTABLE_SHA256, O05_BUILDING_SCHEMA, O05_RUNTIME_CONSUMER_MANIFEST_SHA256,
    O05_RUNTIME_PRODUCER_MANIFEST_SHA256,
};

mod ui_pass;
pub use ui_pass::{
    ActiveMapmodeState, AuthenticatedUiPass, FontBinding, HudCommand, HudCommandExecution,
    HudCommandIdentity, HudPaintCommand, HudPlacement, InlineIconBinding, MapmodeProvenance,
    SpriteLaw, TextHorizontalAlignment, TextLayoutInputs, TextRectOffsets, UiExactPipelineDraw,
    UiPassClip, UiPassOperation, UiPassRect, UI_A729_EXECUTABLE_SHA256, UI_PASS_SCHEMA,
};

mod decimal_u64 {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
            return Err(D::Error::custom("u64 text is not canonical decimal"));
        }
        text.parse::<u64>().map_err(D::Error::custom)
    }
}

pub(crate) mod base64_bytes {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let decoded = decode(&encoded).map_err(D::Error::custom)?;
        if encode(&decoded) != encoded {
            return Err(D::Error::custom(
                "inline byte payload is not canonical base64",
            ));
        }
        Ok(decoded)
    }

    pub(crate) fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            out.push(ALPHABET[(a >> 2) as usize] as char);
            out.push(ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
            if chunk.len() >= 2 {
                out.push(ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() == 3 {
                out.push(ALPHABET[(c & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    pub(crate) fn decode(encoded: &str) -> Result<Vec<u8>, String> {
        let bytes = encoded.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err("inline byte payload base64 length is not divisible by four".into());
        }
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for (group_index, group) in bytes.chunks_exact(4).enumerate() {
            let final_group = group_index + 1 == bytes.len() / 4;
            let a = sextet(group[0])?;
            let b = sextet(group[1])?;
            let c_padding = group[2] == b'=';
            let d_padding = group[3] == b'=';
            if c_padding && !d_padding {
                return Err("inline byte payload base64 has invalid padding".into());
            }
            if (!final_group && (c_padding || d_padding)) || (c_padding && (b & 0x0f) != 0) {
                return Err("inline byte payload base64 has noncanonical padding".into());
            }
            let c = if c_padding { 0 } else { sextet(group[2])? };
            if d_padding && !c_padding && (c & 0x03) != 0 {
                return Err("inline byte payload base64 has noncanonical tail bits".into());
            }
            let d = if d_padding { 0 } else { sextet(group[3])? };
            out.push((a << 2) | (b >> 4));
            if !c_padding {
                out.push((b << 4) | (c >> 2));
            }
            if !d_padding {
                out.push((c << 6) | d);
            }
        }
        Ok(out)
    }

    fn sextet(byte: u8) -> Result<u8, String> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("inline byte payload contains a non-base64 character".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRenderFrame {
    pub schema: String,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub presentation_qpc_frequency_hz: u64,
    pub selected_present: SelectedPresentReceipt,
    pub snow_mud_texture: Option<SnowMudTexturePacket>,
    pub submits: Vec<RenderSubmit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m03_arrows: Option<M03ArrowFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail_supply_arrows: Option<RailSupplyArrowFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub o04_cities: Option<O04CityFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub o05_buildings: Option<O05BuildingFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_ui_pass: Option<AuthenticatedUiPass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnowMudTexturePacket {
    pub schema: String,
    pub executable_sha256: String,
    pub producer_function_rva: u32,
    pub upload_callsite_rva: u32,
    pub upload_target_rva: u32,
    pub producer_instructions_sha256: String,
    pub upload_instructions_sha256: String,
    #[serde(with = "decimal_u64")]
    pub process_start_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub hook_epoch_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub producer_ordinal: u64,
    pub producer_game_hour_raw: i32,
    pub display_game_hour_raw: i32,
    #[serde(with = "decimal_u64")]
    pub producer_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub producer_qpc_frequency_hz: u64,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bytes_per_texel: u32,
    pub format: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
    pub selected_present_sha256: String,
    pub snow_mud_fade_f32_bits: u32,
    pub snow_mud_fade_consumer_count: u32,
    pub snow_mud_fade_consumer_set_sha256: String,
    pub observation_sha256: String,
}

impl SnowMudTexturePacket {
    pub fn computed_observation_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.observation_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    fn validate_against(&self, frame: &ProductionRenderFrame) -> Result<(), String> {
        if self.schema != SNOW_MUD_TEXTURE_SCHEMA
            || self.executable_sha256 != A729_EXECUTABLE_SHA256
            || self.producer_function_rva != SNOW_MUD_PRODUCER_FUNCTION_RVA
            || self.upload_callsite_rva != SNOW_MUD_UPLOAD_CALLSITE_RVA
            || self.upload_target_rva != SNOW_MUD_UPLOAD_TARGET_RVA
            || self.producer_instructions_sha256 != SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256
            || self.upload_instructions_sha256 != SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256
        {
            return Err("SnowMudData producer binary/static identity mismatch".into());
        }
        if self.process_start_nonce == 0
            || self.hook_epoch_nonce == 0
            || self.producer_ordinal == 0
            || self.producer_qpc_ticks == 0
            || self.producer_qpc_frequency_hz != frame.presentation_qpc_frequency_hz
            || self.producer_qpc_ticks > frame.selected_present.selected_qpc_ticks
            || self.display_game_hour_raw != frame.selected_present.selected_game_hour_raw
            || compact_update_index_from_raw(self.display_game_hour_raw) != Some(frame.update_index)
            || compact_update_index_from_raw(self.producer_game_hour_raw).is_none()
            || self.producer_game_hour_raw > self.display_game_hour_raw
        {
            return Err("SnowMudData producer epoch/hour/order identity mismatch".into());
        }
        if self.width != SNOW_MUD_TEXTURE_WIDTH
            || self.height != SNOW_MUD_TEXTURE_HEIGHT
            || self.row_pitch != SNOW_MUD_TEXTURE_ROW_PITCH
            || self.bytes_per_texel != SNOW_MUD_TEXTURE_BYTES_PER_TEXEL
            || self.format != "DXGI_FORMAT_B8G8R8A8_UNORM"
            || self.bytes.len() != SNOW_MUD_TEXTURE_BYTE_LENGTH
        {
            return Err("SnowMudData descriptor/byte-count mismatch".into());
        }
        check_sha256(&self.payload_sha256, Some(&self.bytes))?;
        check_sha256(&self.selected_present_sha256, None)?;
        if self.selected_present_sha256 != frame.selected_present.computed_sha256()? {
            return Err("SnowMudData selected-Present receipt SHA-256 mismatch".into());
        }
        let selected_submit = frame
            .submit(frame.selected_present.selected_candidate_id)
            .ok_or("SnowMudData selected Present has no retained submit")?;
        let fade =
            selected_present_snow_mud_fade_receipt(selected_submit, &self.selected_present_sha256)?;
        if self.snow_mud_fade_f32_bits != fade.fade_f32_bits
            || self.snow_mud_fade_consumer_count != fade.consumer_count
            || self.snow_mud_fade_consumer_set_sha256 != fade.consumer_set_sha256
        {
            return Err("SnowMudData selected-Present fade consumer receipt mismatch".into());
        }
        validate_selected_present_snow_mud_sources(selected_submit, self)?;
        check_sha256(&self.observation_sha256, None)?;
        if self.observation_sha256 != self.computed_observation_sha256()? {
            return Err("SnowMudData observation SHA-256 mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedPresentReceipt {
    pub candidate_count: u32,
    pub candidate_set_sha256: String,
    #[serde(with = "decimal_u64")]
    pub selected_candidate_id: u64,
    pub selected_ordinal: u32,
    pub selected_game_hour_raw: i32,
    #[serde(with = "decimal_u64")]
    pub selected_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub selection_cutoff_qpc_ticks: u64,
    /// Optional authenticated mouse/keyboard sampling receipt bound to this
    /// selected Present. Session identity (process id, start nonce, hook
    /// epoch), catalog digests, and the effective-VFS manifest root are NOT
    /// carried per frame: they are session-sealed once in the embedded
    /// `EarlyRenderCaptureReceipt` of the recorder manifest (packaging
    /// admission proves byte-equality and binds nested identity copies), and
    /// content joins are validated against the ADMITTED root passed explicitly
    /// to `validate`/`decode`/`encode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_input_observation: Option<RecordedInputObservationReceipt>,
}

impl SelectedPresentReceipt {
    pub fn sealed(
        candidate_count: u32,
        candidate_set_sha256: String,
        selected_candidate_id: u64,
        selected_ordinal: u32,
        selected_game_hour_raw: i32,
        selected_qpc_ticks: u64,
        selection_cutoff_qpc_ticks: u64,
    ) -> Self {
        Self {
            candidate_count,
            candidate_set_sha256,
            selected_candidate_id,
            selected_ordinal,
            selected_game_hour_raw,
            selected_qpc_ticks,
            selection_cutoff_qpc_ticks,
            recorded_input_observation: None,
        }
    }

    pub fn computed_sha256(&self) -> Result<String, String> {
        // The optional recorded-input observation is excluded from the
        // receipt preimage: the observation itself names this receipt's hash,
        // so hashing it would be circular. The receipt hash covers the
        // selected-Present facts only.
        let mut canonical = self.clone();
        canonical.recorded_input_observation = None;
        canonical_json_sha256(&canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnowMudReplayReadbackReceipt {
    pub schema: String,
    #[serde(with = "decimal_u64")]
    pub process_start_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub hook_epoch_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub producer_ordinal: u64,
    pub producer_game_hour_raw: i32,
    pub display_game_hour_raw: i32,
    #[serde(with = "decimal_u64")]
    pub selected_candidate_id: u64,
    #[serde(with = "decimal_u64")]
    pub selected_qpc_ticks: u64,
    pub selected_present_sha256: String,
    pub snow_mud_fade_f32_bits: u32,
    pub snow_mud_fade_consumer_count: u32,
    pub snow_mud_fade_consumer_set_sha256: String,
    pub observation_sha256: String,
    pub payload_sha256: String,
    pub readback_sha256: String,
    pub width: u32,
    pub height: u32,
    pub source_row_pitch: u32,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    #[serde(with = "decimal_u64")]
    pub verified_tight_byte_count: u64,
    pub replaced_binding_count: u32,
    pub receipt_sha256: String,
}

impl SnowMudReplayReadbackReceipt {
    pub fn computed_receipt_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.receipt_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    pub fn validate_against(
        &self,
        packet: &SnowMudTexturePacket,
        selected_present: &SelectedPresentReceipt,
    ) -> Result<(), String> {
        if self.schema != SNOW_MUD_H2_READBACK_SCHEMA
            || self.process_start_nonce != packet.process_start_nonce
            || self.hook_epoch_nonce != packet.hook_epoch_nonce
            || self.producer_ordinal != packet.producer_ordinal
            || self.producer_game_hour_raw != packet.producer_game_hour_raw
            || self.display_game_hour_raw != packet.display_game_hour_raw
            || self.selected_candidate_id != selected_present.selected_candidate_id
            || self.selected_qpc_ticks != selected_present.selected_qpc_ticks
            || self.selected_present_sha256 != packet.selected_present_sha256
            || self.selected_present_sha256 != selected_present.computed_sha256()?
            || self.snow_mud_fade_f32_bits != packet.snow_mud_fade_f32_bits
            || self.snow_mud_fade_consumer_count != packet.snow_mud_fade_consumer_count
            || self.snow_mud_fade_consumer_set_sha256 != packet.snow_mud_fade_consumer_set_sha256
            || self.observation_sha256 != packet.observation_sha256
            || self.payload_sha256 != packet.payload_sha256
            || self.readback_sha256 != packet.payload_sha256
        {
            return Err("SnowMudData H2 identity/readback join mismatch".into());
        }
        if self.width != packet.width
            || self.height != packet.height
            || self.source_row_pitch != packet.row_pitch
            || self.mapped_row_pitch < packet.row_pitch
            || self.verified_row_count != packet.height
            || self.verified_tight_byte_count
                != u64::from(packet.row_pitch) * u64::from(packet.height)
            || self.replaced_binding_count == 0
            || self.replaced_binding_count != packet.snow_mud_fade_consumer_count
        {
            return Err("SnowMudData H2 descriptor/coverage/consumer mismatch".into());
        }
        for sha256 in [
            &self.selected_present_sha256,
            &self.snow_mud_fade_consumer_set_sha256,
            &self.observation_sha256,
            &self.payload_sha256,
            &self.readback_sha256,
            &self.receipt_sha256,
        ] {
            check_sha256(sha256, None)?;
        }
        if self.receipt_sha256 != self.computed_receipt_sha256()? {
            return Err("SnowMudData H2 receipt SHA-256 mismatch".into());
        }
        Ok(())
    }
}

pub const OCCUPATION_SECONDARY_H2_READBACK_SCHEMA: &str =
    "hoi4.occupation_secondary_replay_readback.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupationSecondaryReplayReadbackReceipt {
    pub schema: String,
    pub target_binding_identity: String,
    pub upload_order: u32,
    pub selected_present_sha256: String,
    pub payload_sha256: String,
    pub readback_sha256: String,
    pub width: u32,
    pub height: u32,
    pub source_row_pitch: u32,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    #[serde(with = "decimal_u64")]
    pub verified_tight_byte_count: u64,
    pub terrain_t6_binding_count: u32,
    pub water_t3_binding_count: u32,
    pub receipt_sha256: String,
}

impl OccupationSecondaryReplayReadbackReceipt {
    pub fn computed_receipt_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.receipt_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    pub fn validate_against(
        &self,
        packet: &PoliticalTexturePacket,
        selected_present: &SelectedPresentReceipt,
    ) -> Result<(), String> {
        if self.schema != OCCUPATION_SECONDARY_H2_READBACK_SCHEMA
            || packet.kind != PoliticalTextureKind::Secondary
            || packet.target_binding_identity != OCCUPATION_SECONDARY_TARGET_BINDING
            || self.target_binding_identity != packet.target_binding_identity
            || self.upload_order != packet.upload_order
            || self.selected_present_sha256 != selected_present.computed_sha256()?
            || self.payload_sha256 != packet.sha256
            || self.readback_sha256 != packet.sha256
            || self.width != packet.width
            || self.height != packet.height
            || self.source_row_pitch != packet.row_pitch
            || self.mapped_row_pitch < packet.row_pitch
            || self.verified_row_count != packet.height
            || self.verified_tight_byte_count
                != u64::from(packet.row_pitch) * u64::from(packet.height)
            || self.terrain_t6_binding_count == 0
            || self.water_t3_binding_count == 0
        {
            return Err("occupation secondary H2 identity/coverage/consumer mismatch".into());
        }
        for sha256 in [
            &self.selected_present_sha256,
            &self.payload_sha256,
            &self.readback_sha256,
            &self.receipt_sha256,
        ] {
            check_sha256(sha256, None)?;
        }
        if self.receipt_sha256 != self.computed_receipt_sha256()? {
            return Err("occupation secondary H2 receipt SHA-256 mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedInputVariant {
    MouseKeyboard,
    Keyboard,
    Controller,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedInputObservationReceipt {
    pub executable_sha256: String,
    pub process_id: u32,
    #[serde(with = "decimal_u64")]
    pub process_start_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub hook_epoch_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub observation_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub prior_observation_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub selected_present_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub present_sample_started_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub present_sample_completed_qpc_ticks: u64,
    pub selected_present_receipt_sha256: String,
    pub client_extent: [u32; 2],
    pub prior_client_position: [i32; 2],
    pub current_client_position: [i32; 2],
    pub variant: RecordedInputVariant,
    pub authority_manifest_sha256: String,
    pub observation_receipt_sha256: String,
}

impl RecordedInputObservationReceipt {
    pub fn computed_receipt_sha256(&self) -> Result<String, String> {
        let variant = match self.variant {
            RecordedInputVariant::MouseKeyboard => 1u8,
            RecordedInputVariant::Keyboard => 2,
            RecordedInputVariant::Controller => 3,
        };
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.a729.recorded-input-variant-observation.v3\0");
        hasher.update(self.executable_sha256.as_bytes());
        hasher.update(self.process_id.to_le_bytes());
        hasher.update(self.process_start_nonce.to_le_bytes());
        hasher.update(self.hook_epoch_nonce.to_le_bytes());
        hasher.update(self.observation_sequence.to_le_bytes());
        hasher.update(self.prior_observation_qpc_ticks.to_le_bytes());
        hasher.update(self.selected_present_qpc_ticks.to_le_bytes());
        hasher.update(self.present_sample_started_qpc_ticks.to_le_bytes());
        hasher.update(self.present_sample_completed_qpc_ticks.to_le_bytes());
        hasher.update(self.selected_present_receipt_sha256.as_bytes());
        for value in self.client_extent {
            hasher.update(value.to_le_bytes());
        }
        for value in self
            .prior_client_position
            .into_iter()
            .chain(self.current_client_position)
        {
            hasher.update(value.to_le_bytes());
        }
        hasher.update([variant]);
        hasher.update(self.authority_manifest_sha256.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn validate(&self) -> Result<(), String> {
        for value in [
            &self.executable_sha256,
            &self.selected_present_receipt_sha256,
            &self.authority_manifest_sha256,
            &self.observation_receipt_sha256,
        ] {
            check_sha256(value, None)?;
        }
        let inside = |position: [i32; 2], extent: [u32; 2]| {
            position[0] >= 0
                && position[1] >= 0
                && (position[0] as u32) < extent[0]
                && (position[1] as u32) < extent[1]
        };
        if self.executable_sha256 != UI_A729_EXECUTABLE_SHA256
            || self.authority_manifest_sha256 != RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256
            || self.process_id == 0
            || self.process_start_nonce == 0
            || self.hook_epoch_nonce == 0
            || self.observation_sequence == 0
            || self.prior_observation_qpc_ticks == 0
            || self.prior_observation_qpc_ticks >= self.selected_present_qpc_ticks
            || self.selected_present_qpc_ticks > self.present_sample_started_qpc_ticks
            || self.present_sample_started_qpc_ticks > self.present_sample_completed_qpc_ticks
            || self.client_extent[0] == 0
            || self.client_extent[1] == 0
            || self.prior_client_position == self.current_client_position
            || !inside(self.prior_client_position, self.client_extent)
            || !inside(self.current_client_position, self.client_extent)
            || self.computed_receipt_sha256()? != self.observation_receipt_sha256
        {
            return Err(
                "production render recorded-input observation is unauthenticated or non-chronological"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Producer-side attestation envelope retained by capture plumbing (static
/// program promotion, runtime drain tuples). The per-frame wire no longer
/// carries it: the observation rides on `SelectedPresentReceipt` and session
/// identity is sealed once in the recorder manifest's
/// `EarlyRenderCaptureReceipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderAuthority {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_input_observation: Option<RecordedInputObservationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSubmit {
    pub global_order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_carrier: Option<AuthenticatedCameraCarrier>,
    #[serde(default)]
    pub ordered_static_map_programs: Vec<StaticMapPassProgram>,
    #[serde(default)]
    pub authenticated_texture_snapshots: Vec<AuthenticatedTextureSnapshotPacket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s1_shadow: Option<S1ShadowCarrier>,
    pub particles: Vec<ParticlePacket>,
    pub particle_packet_set_sha256: String,
    pub entities: Vec<EntitySubmission>,
    #[serde(default)]
    pub rail_construction_arrows: Vec<RailConstructionArrowVisual>,
    #[serde(default)]
    pub ordered_render_operations: Vec<RenderExecutionRef>,
    pub weather_membership: Vec<WeatherMembership>,
    #[serde(default)]
    pub political_textures: Vec<PoliticalTexturePacket>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A729CameraOwnerKind {
    FrontEnd,
    InGame,
    Third,
}

impl A729CameraOwnerKind {
    /// The a729 vtable RVA this owner kind must carry.
    ///
    /// `pub` because it is the LAW binding two public things - the public
    /// `owner_kind` and the public `owner_vtable_rva` field that
    /// `AuthenticatedCameraCarrier::validate` requires to equal it (see the
    /// two `owner_vtable_rva != owner_kind.expected_vtable_rva()` arms below).
    /// A consumer that builds or checks a lawful carrier has to read the value
    /// somewhere, and the alternative to reading it here is a fresh copy of an
    /// a729 constant in another crate, which the no-hardcoded-game-content rule
    /// forbids.
    ///
    /// This is the one place the LAW is stated, which is not the same as being
    /// the only place the numbers appear: this file's own fixtures write the
    /// InGame RVA as a literal, and the capture producer's hook carries all
    /// three in its owner-dispatch table (`a729_hook.rs`), because it resolves
    /// the kind FROM the observed vtable rather than checking one it was given.
    /// Those are separate readings of the same RE evidence, not consumers of
    /// this law - and new consumers should call this instead of adding to them.
    pub fn expected_vtable_rva(self) -> u32 {
        match self {
            Self::FrontEnd => 0x0293_3710,
            Self::InGame => 0x0295_2510,
            Self::Third => 0x0299_3698,
        }
    }

    fn expected_event_list_offset(self) -> u32 {
        match self {
            Self::FrontEnd => 0x5f0,
            Self::InGame => 0x758,
            Self::Third => 0x648,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct A729CameraInputEvent {
    pub node_ordinal: u32,
    pub node_kind_i32: i32,
    pub consumed_before_u8: u8,
    pub consumed_after_u8: u8,
    pub event_type_i32: i32,
    pub signed_delta_i32: i32,
    pub embedded_event_16_bytes: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct A729CameraMapPick {
    pub call_ordinal: u32,
    pub mode_u8: u8,
    pub screen_xy_f32_bits: [u32; 2],
    pub world_xyz_f32_bits: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct A729CameraEventBatch {
    pub producer_rva: u32,
    pub owner_kind: A729CameraOwnerKind,
    pub owner_vtable_rva: u32,
    pub action_source_owner_offset: u32,
    pub cursor_source_owner_offset: u32,
    pub event_list_owner_offset: u32,
    pub frame_scalar_f32_bits: Option<u32>,
    pub producer_flag_0_u8: u8,
    pub producer_flag_1_u8: u8,
    pub input_events: Vec<A729CameraInputEvent>,
    pub map_picks: Vec<A729CameraMapPick>,
    pub current_eye_before_f32_bits: [u32; 3],
    pub target_eye_before_f32_bits: [u32; 3],
    pub target_look_at_before_f32_bits: [u32; 3],
    pub current_eye_after_f32_bits: [u32; 3],
    pub target_eye_after_f32_bits: [u32; 3],
    pub target_look_at_after_f32_bits: [u32; 3],
}

impl A729CameraEventBatch {
    fn validate(&self, carrier_owner: A729CameraOwnerKind) -> Result<(), String> {
        if self.owner_kind != carrier_owner
            || self.owner_vtable_rva != self.owner_kind.expected_vtable_rva()
            || self.action_source_owner_offset != 0x508
            || self.cursor_source_owner_offset != 0x500
            || self.event_list_owner_offset != self.owner_kind.expected_event_list_offset()
            || self.input_events.len() > 4_096
            || self.map_picks.len() > 4_096
        {
            return Err("camera event batch owner/layout/bound invariant mismatch".into());
        }
        match self.producer_rva {
            A729_CAMERA_UPDATE_RVA if self.frame_scalar_f32_bits.is_some() => {}
            A729_CAMERA_VERTICAL_EVENT_PRODUCER_RVA
                if self.owner_kind == A729CameraOwnerKind::InGame
                    && self.frame_scalar_f32_bits.is_none()
                    && self.producer_flag_1_u8 == 0 => {}
            _ => return Err("camera event batch producer identity mismatch".into()),
        }
        for (ordinal, event) in self.input_events.iter().enumerate() {
            if event.node_ordinal != ordinal as u32
                || event.consumed_before_u8 > 1
                || event.consumed_after_u8 > 1
                || event.event_type_i32
                    != i32::from_le_bytes(event.embedded_event_16_bytes[4..8].try_into().unwrap())
                || event.signed_delta_i32
                    != i32::from_le_bytes(event.embedded_event_16_bytes[8..12].try_into().unwrap())
            {
                return Err("camera event row identity/raw-byte invariant mismatch".into());
            }
        }
        for (ordinal, pick) in self.map_picks.iter().enumerate() {
            if pick.call_ordinal != ordinal as u32 || pick.mode_u8 > 1 {
                return Err("camera map-pick order/mode invariant mismatch".into());
            }
        }
        Ok(())
    }

    fn update_digest(&self, hasher: &mut Sha256) {
        hasher.update(self.producer_rva.to_le_bytes());
        hasher.update([match self.owner_kind {
            A729CameraOwnerKind::FrontEnd => 0,
            A729CameraOwnerKind::InGame => 1,
            A729CameraOwnerKind::Third => 2,
        }]);
        for value in [
            self.owner_vtable_rva,
            self.action_source_owner_offset,
            self.cursor_source_owner_offset,
            self.event_list_owner_offset,
        ] {
            hasher.update(value.to_le_bytes());
        }
        match self.frame_scalar_f32_bits {
            Some(value) => {
                hasher.update([1]);
                hasher.update(value.to_le_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update([self.producer_flag_0_u8, self.producer_flag_1_u8]);
        hasher.update((self.input_events.len() as u32).to_le_bytes());
        for event in &self.input_events {
            hasher.update(event.node_ordinal.to_le_bytes());
            hasher.update(event.node_kind_i32.to_le_bytes());
            hasher.update([event.consumed_before_u8, event.consumed_after_u8]);
            hasher.update(event.event_type_i32.to_le_bytes());
            hasher.update(event.signed_delta_i32.to_le_bytes());
            hasher.update(event.embedded_event_16_bytes);
        }
        hasher.update((self.map_picks.len() as u32).to_le_bytes());
        for pick in &self.map_picks {
            hasher.update(pick.call_ordinal.to_le_bytes());
            hasher.update([pick.mode_u8]);
            for value in pick
                .screen_xy_f32_bits
                .into_iter()
                .chain(pick.world_xyz_f32_bits)
            {
                hasher.update(value.to_le_bytes());
            }
        }
        for row in [
            self.current_eye_before_f32_bits,
            self.target_eye_before_f32_bits,
            self.target_look_at_before_f32_bits,
            self.current_eye_after_f32_bits,
            self.target_eye_after_f32_bits,
            self.target_look_at_after_f32_bits,
        ] {
            for value in row {
                hasher.update(value.to_le_bytes());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedCameraCarrier {
    pub schema: String,
    pub executable_sha256: String,
    pub capture_boundary_rva: u32,
    pub camera_update_rva: u32,
    pub carrier_ordinal: u8,
    pub carrier_offset: u32,
    pub owner_kind: A729CameraOwnerKind,
    pub owner_vtable_rva: u32,
    pub render_view_436_bytes: Vec<u8>,
    pub canonical_camera_root_436_bytes: Vec<u8>,
    pub previous_current_eye_f32_bits: [u32; 3],
    pub current_eye_f32_bits: [u32; 3],
    pub current_look_at_f32_bits: [u32; 3],
    pub target_eye_f32_bits: [u32; 3],
    pub target_look_at_f32_bits: [u32; 3],
    pub viewport_i32: [i32; 4],
    pub min_height_i32: i32,
    pub max_height_i32: i32,
    pub update_produced_u8: u8,
    pub special_free_state_u8: u8,
    pub ramp_f32_bits: u32,
    pub wrap_branch_u8: u8,
    #[serde(default)]
    pub camera_event_batches: Vec<A729CameraEventBatch>,
    pub payload_sha256: String,
}

impl AuthenticatedCameraCarrier {
    pub fn view_projection_f32_bits(&self) -> [u32; 16] {
        std::array::from_fn(|index| {
            let start = 0x80 + index * 4;
            u32::from_le_bytes(
                self.canonical_camera_root_436_bytes[start..start + 4]
                    .try_into()
                    .expect("validated C01 carrier length"),
            )
        })
    }

    pub fn computed_payload_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.schema.as_bytes());
        hasher.update(self.executable_sha256.as_bytes());
        for value in [
            self.capture_boundary_rva,
            self.camera_update_rva,
            u32::from(self.carrier_ordinal),
            self.carrier_offset,
            self.owner_vtable_rva,
        ] {
            hasher.update(value.to_le_bytes());
        }
        hasher.update([match self.owner_kind {
            A729CameraOwnerKind::FrontEnd => 0u8,
            A729CameraOwnerKind::InGame => 1u8,
            A729CameraOwnerKind::Third => 2u8,
        }]);
        hasher.update(&self.render_view_436_bytes);
        hasher.update(&self.canonical_camera_root_436_bytes);
        for row in [
            self.previous_current_eye_f32_bits,
            self.current_eye_f32_bits,
            self.current_look_at_f32_bits,
            self.target_eye_f32_bits,
            self.target_look_at_f32_bits,
        ] {
            for value in row {
                hasher.update(value.to_le_bytes());
            }
        }
        for value in self.viewport_i32 {
            hasher.update(value.to_le_bytes());
        }
        hasher.update(self.min_height_i32.to_le_bytes());
        hasher.update(self.max_height_i32.to_le_bytes());
        hasher.update([self.update_produced_u8, self.special_free_state_u8]);
        hasher.update(self.ramp_f32_bits.to_le_bytes());
        hasher.update([self.wrap_branch_u8]);
        hasher.update((self.camera_event_batches.len() as u32).to_le_bytes());
        for batch in &self.camera_event_batches {
            batch.update_digest(&mut hasher);
        }
        format!("{:x}", hasher.finalize())
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != A729_CAMERA_TEMPORAL_SCHEMA
            || self.executable_sha256 != A729_CAMERA_EXECUTABLE_SHA256
            || self.capture_boundary_rva != A729_CAMERA_ENVELOPE_RVA
            || self.camera_update_rva != A729_CAMERA_UPDATE_RVA
        {
            return Err("camera carrier authority/identity mismatch".into());
        }
        let expected_offset = A729_CAMERA_CARRIER_OFFSETS
            .get(usize::from(self.carrier_ordinal))
            .copied()
            .ok_or_else(|| "camera carrier ordinal is outside 0..=2".to_string())?;
        if self.carrier_offset != expected_offset
            || self.owner_vtable_rva != self.owner_kind.expected_vtable_rva()
            || self.render_view_436_bytes.len() != A729_CAMERA_RENDER_VIEW_BYTES
            || self.canonical_camera_root_436_bytes.len() != A729_CAMERA_RENDER_VIEW_BYTES
            || self.update_produced_u8 > 1
            || self.special_free_state_u8 > 1
            || self.wrap_branch_u8 > 1
            || self.min_height_i32 > self.max_height_i32
            || self.camera_event_batches.len() > 1_024
        {
            return Err("camera carrier layout/state invariant mismatch".into());
        }
        let mut prior_after = None;
        for batch in &self.camera_event_batches {
            batch.validate(self.owner_kind)?;
            if let Some((current_eye, target_eye, target_look_at)) = prior_after {
                if batch.current_eye_before_f32_bits != current_eye
                    || batch.target_eye_before_f32_bits != target_eye
                    || batch.target_look_at_before_f32_bits != target_look_at
                {
                    return Err("camera event batches do not form one exact state chain".into());
                }
            }
            prior_after = Some((
                batch.current_eye_after_f32_bits,
                batch.target_eye_after_f32_bits,
                batch.target_look_at_after_f32_bits,
            ));
        }
        if let Some((current_eye, target_eye, target_look_at)) = prior_after {
            if current_eye != self.current_eye_f32_bits
                || target_eye != self.target_eye_f32_bits
                || target_look_at != self.target_look_at_f32_bits
            {
                return Err(
                    "camera event producer result differs from the joined envelope state".into(),
                );
            }
        }
        let carrier_vec3 = |offset: usize| -> [u32; 3] {
            std::array::from_fn(|index| {
                let start = offset + index * 4;
                u32::from_le_bytes(
                    self.canonical_camera_root_436_bytes[start..start + 4]
                        .try_into()
                        .expect("validated C01 carrier length"),
                )
            })
        };
        if carrier_vec3(0x180) != self.current_eye_f32_bits
            || carrier_vec3(0x18c) != self.current_look_at_f32_bits
        {
            return Err("camera carrier eye/look-at disagrees with C00 root fields".into());
        }
        if self.carrier_ordinal == 0
            && self.render_view_436_bytes != self.canonical_camera_root_436_bytes
        {
            return Err("camera ordinal-zero carrier differs from canonical root view".into());
        }
        check_sha256(&self.payload_sha256, None)?;
        if self.payload_sha256 != self.computed_payload_sha256() {
            return Err("camera carrier payload SHA-256 mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapPassProgram {
    pub program_ordinal: u32,
    pub pass_token: String,
    pub ordered_content_keys: Vec<String>,
    pub ordered_operations: Vec<StaticMapOperation>,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderExecutionRef {
    Static { operation_ordinal: u32 },
    Runtime { output: RenderOutputRef },
    M03 { output: M03ArrowDrawRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapOperation {
    pub operation_ordinal: u32,
    pub payload_sha256: String,
    pub command: StaticMapCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticMapCommand {
    ClearColor {
        target: RenderTargetDesc,
        rgba_f32_bits: [u32; 4],
    },
    ClearDepthStencil {
        target: RenderTargetDesc,
        clear_depth: bool,
        clear_stencil: bool,
        depth_f32_bits: u32,
        stencil_u8: u8,
    },
    Draw {
        draw: StaticMapDraw,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapDraw {
    pub draw_token: String,
    pub shader_program: ShaderProgram,
    pub ordered_shader_resources: Vec<StaticShaderResourceBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: StaticGeometrySource,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapnameD80State {
    Missing,
    LegacyUnstamped,
    Authenticated,
}

impl MapnameD80State {
    pub fn merge(self, next: Self) -> Result<Self, String> {
        match (self, next) {
            (Self::Missing, state) | (state, Self::Missing) => Ok(state),
            (Self::LegacyUnstamped, Self::LegacyUnstamped) => Ok(Self::LegacyUnstamped),
            (Self::Authenticated, Self::Authenticated) => Ok(Self::Authenticated),
            _ => Err(
                "static map programs mix legacy-unstamped and authenticated mapname draws".into(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticShaderResourceBinding {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub source: StaticShaderResourceSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticShaderResourceSource {
    UsedContent {
        content: UsedContentKey,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    /// Exact authenticated draw-time resource snapshot for an engine-generated
    AuthenticatedSnapshot {
        snapshot_sha256: String,
        view: ShaderResourceViewDesc,
    },
    PreparedTarget {
        target: PreparedRenderTarget,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
        producer: RenderExecutionRef,
    },
    SnowMudTexture {
        #[serde(with = "decimal_u64")]
        producer_ordinal: u64,
        payload_sha256: String,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    PoliticalTexture {
        texture_kind: PoliticalTextureKind,
        target_binding_identity: String,
        upload_order: u32,
        payload_sha256: String,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    TiledLightIndex {
        #[serde(with = "decimal_u64")]
        writer_index: u64,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    TiledLightData {
        #[serde(with = "decimal_u64")]
        writer_index: u64,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedTextureSnapshotPacket {
    pub snapshot_sha256: String,
    pub resource: TextureResourceDesc,
}

impl AuthenticatedTextureSnapshotPacket {
    pub fn sealed(resource: TextureResourceDesc) -> Result<Self, String> {
        validate_authenticated_snapshot_texture_resource(&resource)?;
        Ok(Self {
            snapshot_sha256: authenticated_texture_snapshot_sha256(&resource)?,
            resource,
        })
    }

    fn validate(&self) -> Result<(), String> {
        validate_authenticated_snapshot_texture_resource(&self.resource)?;
        check_sha256(&self.snapshot_sha256, None)?;
        if self.snapshot_sha256 != authenticated_texture_snapshot_sha256(&self.resource)? {
            return Err("authenticated texture snapshot packet SHA-256 mismatch".into());
        }
        Ok(())
    }
}

fn authenticated_texture_snapshot_sha256(resource: &TextureResourceDesc) -> Result<String, String> {
    let canonical = serde_json::to_vec(resource).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.authenticated-draw-time-texture-snapshot.v1\0");
    hasher.update(canonical);
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticGeometrySource {
    Exact {
        geometry: GeometryBinding,
    },
    InterpreterOutput {
        interpreter_token: String,
        ordered_input_content_keys: Vec<String>,
        ordered_vertex_buffers: Vec<InterpreterVertexBufferDesc>,
        index_buffer: Option<InterpreterIndexBufferDesc>,
        stream_output_targets: Vec<StreamOutputTargetBinding>,
        draw: DrawCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterVertexBufferDesc {
    pub input_slot: u32,
    pub stride_bytes: u32,
    pub byte_offset: u32,
    pub vertex_count: u32,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterIndexBufferDesc {
    pub format: IndexFormat,
    pub byte_offset: u32,
    pub index_count: u32,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticlePacket {
    #[serde(with = "decimal_u64")]
    pub effect_instance_id: u64,
    pub registration_token: String,
    pub resolved_definition_sha256: String,
    pub content_identity: ParticleContentIdentity,
    pub content_identity_sha256: String,
    pub bucket: u32,
    pub packet_ordinal: u32,
    pub draw_order: u32,
    pub ribbon_tail_order: Option<u32>,
    #[serde(with = "base64_bytes")]
    pub canonical_packet_112_bytes: Vec<u8>,
    pub canonical_packet_sha256: String,
    pub packet_matrix_4x4_f32_bits: [u32; 16],
    pub live_count: u32,
    pub record_stride: u32,
    pub max_count: u32,
    #[serde(with = "base64_bytes")]
    pub ordered_gpu_instance_records: Vec<u8>,
    pub gpu_instance_records_sha256: String,
    pub elapsed_f32_bits: u32,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub atlas_rows: u32,
    pub atlas_columns: u32,
    pub scale_f32_bits: u32,
    pub flags_u16: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticleContentIdentity {
    pub definition_content: UsedContentKey,
    pub subsystem_selector_u32: u32,
    pub texture_content: UsedContentKey,
    pub authored_shader_token: String,
    pub engine_effect_token: String,
    pub atlas_columns_u32: u32,
    pub atlas_rows_u32: u32,
    pub atlas_half_pixel_width_f32_bits: u32,
    pub atlas_half_pixel_height_f32_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderProgram {
    pub source: UsedContentKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_registration_root_rva: Option<u32>,
    pub effect_token: String,
    pub vertex_entry_token: String,
    pub pixel_entry_token: String,
    pub ordered_defines: Vec<ShaderDefine>,
    pub compiled_vertex_shader: CompiledShaderPayload,
    pub compiled_pixel_shader: CompiledShaderPayload,
    pub geometry_shader: Option<AdditionalShaderStage>,
    pub hull_shader: Option<AdditionalShaderStage>,
    pub domain_shader: Option<AdditionalShaderStage>,
    pub compute_shader: Option<AdditionalShaderStage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdditionalShaderStage {
    pub entry_token: String,
    pub compiled: CompiledShaderPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderDefine {
    pub name: String,
    pub value: String,
    pub scope: ShaderDefineScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderDefineScope {
    EngineGlobal,
    Effect,
    EntryPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledShaderPayload {
    pub compiler_identity: String,
    pub target_profile: String,
    pub compiler_flags_u64: u64,
    #[serde(with = "base64_bytes")]
    pub bytecode: Vec<u8>,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderStage {
    Vertex,
    Pixel,
    Geometry,
    Hull,
    Domain,
    Compute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureBinding {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub content: UsedContentKey,
    pub resource: TextureResourceDesc,
    pub view: ShaderResourceViewDesc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureResourceDesc {
    pub dimension: TextureResourceDimension,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub dxgi_format_u32: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub usage_u32: u32,
    pub bind_flags_u32: u32,
    pub cpu_access_flags_u32: u32,
    pub misc_flags_u32: u32,
    pub ordered_subresources: Vec<TextureSubresourcePayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureResourceDimension {
    Texture1d,
    Texture2d,
    Texture3d,
    TextureCube,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureSubresourcePayload {
    pub subresource_index: u32,
    pub mip_level: u32,
    pub array_slice: u32,
    pub row_pitch: u32,
    pub slice_pitch: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShaderResourceViewDesc {
    Buffer {
        dxgi_format_u32: u32,
        first_element: u32,
        element_count: u32,
        flags_u32: u32,
    },
    Texture1d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    Texture1dArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    Texture2dArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled {
        dxgi_format_u32: u32,
    },
    Texture2dMultisampledArray {
        dxgi_format_u32: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture3d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    TextureCube {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    TextureCubeArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_face: u32,
        cube_count: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstantBufferBinding {
    pub stage_mask_u8: u8,
    pub slot: u8,
    pub byte_offset: u32,
    pub byte_count: u32,
    pub raw_bytes: Vec<u8>,
    pub sha256: String,
    pub semantic_layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnowMudFadeReceipt {
    pub fade_f32_bits: u32,
    pub consumer_count: u32,
    pub consumer_set_sha256: String,
}

#[derive(Serialize)]
struct SnowMudFadeConsumerHashRow<'a> {
    selected_present_sha256: &'a str,
    operation_ordinal: u32,
    draw_token: &'a str,
    draw_payload_sha256: &'a str,
    shader_source_payload_sha256: &'a str,
    pixel_shader_sha256: &'a str,
    texture_slot: u8,
    constant_buffer_slot: u8,
    constant_buffer_sha256: &'a str,
    constant_buffer_layout_sha256: String,
    variable_start_offset: u32,
    fade_f32_bits: u32,
}

#[derive(Clone, Copy)]
struct ReflectedVariableRange {
    buffer_byte_count: u32,
    start_offset: u32,
    byte_count: u32,
}

fn parse_decimal_component<'a>(input: &'a str, delimiter: u8) -> Result<(u32, &'a str), String> {
    let bytes = input.as_bytes();
    let end = bytes
        .iter()
        .position(|byte| *byte == delimiter)
        .ok_or("reflected constant-buffer layout is truncated")?;
    let digits = &input[..end];
    if digits.is_empty()
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return Err("reflected constant-buffer layout has a non-canonical decimal".into());
    }
    let value = digits
        .parse::<u32>()
        .map_err(|_| "reflected constant-buffer decimal exceeds u32")?;
    Ok((value, &input[end + 1..]))
}

fn parse_length_prefixed_text(input: &str) -> Result<(&str, &str), String> {
    let (length, rest) = parse_decimal_component(input, b':')?;
    let length = usize::try_from(length).map_err(|_| "reflected text length exceeds usize")?;
    let text = rest
        .get(..length)
        .ok_or("reflected length-prefixed text is truncated")?;
    Ok((text, &rest[length..]))
}

fn reflected_variable_range(
    layout: &str,
    wanted_name: &str,
) -> Result<Option<ReflectedVariableRange>, String> {
    let mut rest = layout
        .strip_prefix("d3d11-cbuffer-v1[")
        .ok_or("constant-buffer semantic layout is not d3d11-cbuffer-v1")?;
    let (_buffer_name, next) = parse_length_prefixed_text(rest)?;
    rest = next
        .strip_prefix(';')
        .ok_or("reflected constant-buffer descriptor is truncated")?;
    let (_buffer_type, next) = parse_decimal_component(rest, b';')?;
    let (buffer_byte_count, next) = parse_decimal_component(next, b';')?;
    let header_end = next
        .bytes()
        .position(|byte| byte == b';' || byte == b']')
        .ok_or("reflected constant-buffer flags are truncated")?;
    let flags = &next[..header_end];
    if flags.is_empty()
        || !flags.bytes().all(|byte| byte.is_ascii_digit())
        || (flags.len() > 1 && flags.starts_with('0'))
    {
        return Err("reflected constant-buffer flags are not canonical decimal".into());
    }
    let _buffer_flags = flags
        .parse::<u32>()
        .map_err(|_| "reflected constant-buffer flags exceed u32")?;
    let mut rest = &next[header_end..];
    if rest == "]" {
        return Ok(None);
    }
    rest = &rest[1..];
    let mut found = None;
    loop {
        if rest == "]" {
            return Ok(found);
        }
        let (name, next) = parse_length_prefixed_text(rest)?;
        rest = next
            .strip_prefix(':')
            .ok_or("reflected variable offset is truncated")?;
        let (start_offset, next) = parse_decimal_component(rest, b':')?;
        let (byte_count, next) = parse_decimal_component(next, b':')?;
        let (_flags, next) = parse_decimal_component(next, b':')?;
        let (_type_token, next) = parse_length_prefixed_text(next)?;
        if name == wanted_name {
            if found.is_some() {
                return Err("reflected constant buffer repeats the SnowMudFade variable".into());
            }
            found = Some(ReflectedVariableRange {
                buffer_byte_count,
                start_offset,
                byte_count,
            });
        }
        rest = next
            .strip_prefix(';')
            .or_else(|| if next == "]" { Some(next) } else { None })
            .ok_or("reflected constant-buffer variable separator is invalid")?;
    }
}

pub fn selected_present_snow_mud_fade_receipt(
    submit: &RenderSubmit,
    selected_present_sha256: &str,
) -> Result<SnowMudFadeReceipt, String> {
    check_sha256(selected_present_sha256, None)?;
    let mut rows = Vec::new();
    let mut common_bits = None;
    for program in &submit.ordered_static_map_programs {
        for operation in &program.ordered_operations {
            let StaticMapCommand::Draw { draw } = &operation.command else {
                continue;
            };
            let mut snow_bindings = draw
                .ordered_shader_resources
                .iter()
                .filter(|binding| binding.semantic_token == SNOW_MUD_TEXTURE_SEMANTIC);
            let Some(snow_binding) = snow_bindings.next() else {
                continue;
            };
            if snow_bindings.next().is_some() || snow_binding.stage != ShaderStage::Pixel {
                return Err("selected SnowMud draw repeats or mis-stages SnowMudTexture".into());
            }
            let mut matching = Vec::new();
            for binding in draw
                .ordered_constant_buffers
                .iter()
                .filter(|binding| binding.stage_mask_u8 == 0b0000_0010)
            {
                let Some(variable) =
                    reflected_variable_range(&binding.semantic_layout, SNOW_MUD_FADE_VARIABLE)?
                else {
                    continue;
                };
                if binding.byte_offset != 0
                    || binding.byte_count != variable.buffer_byte_count
                    || binding.raw_bytes.len() != binding.byte_count as usize
                    || variable.byte_count != 16
                    || variable
                        .start_offset
                        .checked_add(SNOW_MUD_FADE_COMPONENT_BYTE_OFFSET + 4)
                        .is_none_or(|end| end > binding.byte_count)
                {
                    return Err("selected SnowMud draw has an invalid reflected fade range".into());
                }
                let start =
                    usize::try_from(variable.start_offset + SNOW_MUD_FADE_COMPONENT_BYTE_OFFSET)
                        .map_err(|_| "SnowMudFade offset exceeds usize")?;
                let bits = u32::from_le_bytes(
                    binding.raw_bytes[start..start + 4]
                        .try_into()
                        .expect("range checked"),
                );
                let value = f32::from_bits(bits);
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err("selected SnowMudFade is not finite in [0,1]".into());
                }
                matching.push((binding, variable, bits));
            }
            if matching.len() != 1 {
                return Err("selected SnowMud draw lacks one exact reflected fade consumer".into());
            }
            let (binding, variable, bits) = matching[0];
            if common_bits.replace(bits).is_some_and(|prior| prior != bits) {
                return Err("selected SnowMud draws disagree on SnowMudFade bits".into());
            }
            rows.push(SnowMudFadeConsumerHashRow {
                selected_present_sha256,
                operation_ordinal: operation.operation_ordinal,
                draw_token: &draw.draw_token,
                draw_payload_sha256: &draw.payload_sha256,
                shader_source_payload_sha256: &draw.shader_program.source.payload_sha256,
                pixel_shader_sha256: &draw.shader_program.compiled_pixel_shader.sha256,
                texture_slot: snow_binding.slot,
                constant_buffer_slot: binding.slot,
                constant_buffer_sha256: &binding.sha256,
                constant_buffer_layout_sha256: format!(
                    "{:x}",
                    Sha256::digest(binding.semantic_layout.as_bytes())
                ),
                variable_start_offset: variable.start_offset,
                fade_f32_bits: bits,
            });
        }
    }
    let fade_f32_bits =
        common_bits.ok_or("selected submit has no static SnowMudTexture fade consumer")?;
    let consumer_count =
        u32::try_from(rows.len()).map_err(|_| "SnowMudFade consumer count exceeds u32")?;
    let consumer_set_sha256 = canonical_json_sha256(&rows)?;
    Ok(SnowMudFadeReceipt {
        fade_f32_bits,
        consumer_count,
        consumer_set_sha256,
    })
}

fn validate_selected_present_snow_mud_sources(
    submit: &RenderSubmit,
    packet: &SnowMudTexturePacket,
) -> Result<(), String> {
    let mut joined = 0u32;
    for program in &submit.ordered_static_map_programs {
        for operation in &program.ordered_operations {
            let StaticMapCommand::Draw { draw } = &operation.command else {
                continue;
            };
            for binding in &draw.ordered_shader_resources {
                match (&binding.semantic_token[..], &binding.source) {
                    (
                        SNOW_MUD_TEXTURE_SEMANTIC,
                        StaticShaderResourceSource::SnowMudTexture {
                            producer_ordinal,
                            payload_sha256,
                            resource,
                            ..
                        },
                    ) if *producer_ordinal == packet.producer_ordinal
                        && payload_sha256 == &packet.payload_sha256
                        && resource.width == packet.width
                        && resource.height == packet.height
                        && resource.dxgi_format_u32 == 87
                        && resource.sample_count == 1 =>
                    {
                        joined = joined
                            .checked_add(1)
                            .ok_or("SnowMudTexture source consumer count overflow")?;
                    }
                    (SNOW_MUD_TEXTURE_SEMANTIC, _) => {
                        return Err("selected SnowMudTexture consumer is not joined to H".into())
                    }
                    (_, StaticShaderResourceSource::SnowMudTexture { .. }) => {
                        return Err(
                            "SnowMudTexture H source has a foreign reflected semantic".into()
                        )
                    }
                    _ => {}
                }
            }
        }
    }
    if joined == 0 || joined != packet.snow_mud_fade_consumer_count {
        return Err("SnowMudTexture H source/fade consumer cardinality mismatch".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineDesc {
    pub topology: PrimitiveTopology,
    pub blend: BlendDesc,
    pub depth: DepthDesc,
    pub rasterizer: RasterizerDesc,
    pub samplers: Vec<SamplerDesc>,
    pub input_layout: Vec<InputElementDesc>,
    pub index_format: IndexFormat,
    pub static_indices_u16: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveTopology {
    TriangleList,
    TriangleStrip,
    LineList,
    LineStrip,
    PointList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendDesc {
    pub alpha_to_coverage_enabled: bool,
    pub independent_blend_enabled: bool,
    pub enabled: bool,
    pub source_color: BlendFactor,
    pub destination_color: BlendFactor,
    pub color_operation: BlendOperation,
    pub source_alpha: BlendFactor,
    pub destination_alpha: BlendFactor,
    pub alpha_operation: BlendOperation,
    pub write_mask_rgba_u8: u8,
    pub additional_targets: Vec<RenderTargetBlendDesc>,
    pub blend_factor_rgba_f32_bits: [u32; 4],
    pub sample_mask_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetBlendDesc {
    pub enabled: bool,
    pub source_color: BlendFactor,
    pub destination_color: BlendFactor,
    pub color_operation: BlendOperation,
    pub source_alpha: BlendFactor,
    pub destination_alpha: BlendFactor,
    pub alpha_operation: BlendOperation,
    pub write_mask_rgba_u8: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendFactor {
    Zero,
    One,
    SourceAlpha,
    InverseSourceAlpha,
    DestinationAlpha,
    InverseDestinationAlpha,
    SourceColor,
    InverseSourceColor,
    DestinationColor,
    InverseDestinationColor,
    SourceAlphaSaturate,
    BlendFactor,
    InverseBlendFactor,
    Source1Color,
    InverseSource1Color,
    Source1Alpha,
    InverseSource1Alpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Minimum,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthDesc {
    pub enabled: bool,
    pub write_enabled: bool,
    pub comparison: Comparison,
    pub stencil_enabled: bool,
    pub stencil_read_mask_u8: u8,
    pub stencil_write_mask_u8: u8,
    pub front_face: StencilFaceDesc,
    pub back_face: StencilFaceDesc,
    pub stencil_reference_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StencilFaceDesc {
    pub fail_operation: StencilOperation,
    pub depth_fail_operation: StencilOperation,
    pub pass_operation: StencilOperation,
    pub comparison: Comparison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StencilOperation {
    Keep,
    Zero,
    Replace,
    IncrementSaturate,
    DecrementSaturate,
    Invert,
    Increment,
    Decrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterizerDesc {
    pub fill: FillMode,
    pub cull: CullMode,
    pub front_counter_clockwise: bool,
    pub depth_clip_enabled: bool,
    pub scissor_enabled: bool,
    pub depth_bias_i32: i32,
    pub depth_bias_clamp_f32_bits: u32,
    pub slope_scaled_depth_bias_f32_bits: u32,
    pub multisample_enabled: bool,
    pub antialiased_line_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    Solid,
    Wireframe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CullMode {
    None,
    Front,
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplerDesc {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub min_filter: Filter,
    pub mag_filter: Filter,
    pub mip_filter: Filter,
    pub address_u: AddressMode,
    pub address_v: AddressMode,
    pub address_w: AddressMode,
    pub comparison: Option<Comparison>,
    pub mip_lod_bias_f32_bits: u32,
    pub max_anisotropy: u32,
    pub border_color_rgba_f32_bits: [u32; 4],
    pub min_lod_f32_bits: u32,
    pub max_lod_f32_bits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Point,
    Linear,
    Anisotropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressMode {
    Wrap,
    Mirror,
    Clamp,
    Border,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputElementDesc {
    pub semantic_token: String,
    pub semantic_index: u32,
    pub dxgi_format_u32: u32,
    pub input_slot: u32,
    pub aligned_byte_offset: u32,
    pub per_instance: bool,
    pub instance_step_rate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexFormat {
    R16Uint,
    R32Uint,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryBinding {
    pub ordered_vertex_buffers: Vec<VertexBufferBinding>,
    pub index_buffer: Option<IndexBufferBinding>,
    pub stream_output_targets: Vec<StreamOutputTargetBinding>,
    pub draw: DrawCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamOutputTargetBinding {
    pub slot: u32,
    pub buffer_identity: String,
    pub buffer_byte_width: u32,
    pub byte_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexBufferBinding {
    pub input_slot: u32,
    pub stride_bytes: u32,
    pub byte_offset: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexBufferBinding {
    pub format: IndexFormat,
    pub byte_offset: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BufferSource {
    Inline {
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
        sha256: String,
    },
    UsedContentRange {
        content: UsedContentKey,
        source_byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
    },
    ParticleInstanceRecords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawCommand {
    Draw {
        vertex_count: u32,
        start_vertex: u32,
    },
    DrawIndexed {
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
    },
    DrawInstanced {
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex: u32,
        start_instance: u32,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index: u32,
        base_vertex: i32,
        start_instance: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderOutputState {
    pub ordered_color_targets: Vec<RenderTargetDesc>,
    pub depth_target: Option<RenderTargetDesc>,
    pub viewport: ViewportDesc,
    pub scissor: Option<ScissorDesc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetDesc {
    /// swapchain ownership or an opaque attachment ordinal; semantic scene,
    pub prepared_target: PreparedRenderTarget,
    pub width: u32,
    pub height: u32,
    pub dxgi_format_u32: u32,
    pub mip_level: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub persistent_content: Option<UsedContentKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_resource: Option<RenderTargetExactResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetExactResource {
    pub resource: RenderTargetResourceDesc,
    pub ordered_views: Vec<RenderTargetViewDesc>,
    pub ordered_initial_subresources: Vec<RenderTargetInitialSubresource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetResourceDesc {
    pub dimension: TextureResourceDimension,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub dxgi_format_u32: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub usage_u32: u32,
    pub bind_flags_u32: u32,
    pub cpu_access_flags_u32: u32,
    pub misc_flags_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetViewDesc {
    RenderTarget {
        dxgi_format_u32: u32,
        view: RenderTargetAttachmentView,
    },
    DepthStencil {
        dxgi_format_u32: u32,
        flags_u32: u32,
        view: DepthStencilAttachmentView,
    },
    ShaderResource {
        view: ShaderResourceViewDesc,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetAttachmentView {
    Texture1d {
        mip_slice: u32,
    },
    Texture1dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        mip_slice: u32,
    },
    Texture2dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled,
    Texture2dMultisampledArray {
        first_array_slice: u32,
        array_size: u32,
    },
    Texture3d {
        mip_slice: u32,
        first_w_slice: u32,
        w_size: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum DepthStencilAttachmentView {
    Texture1d {
        mip_slice: u32,
    },
    Texture1dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        mip_slice: u32,
    },
    Texture2dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled,
    Texture2dMultisampledArray {
        first_array_slice: u32,
        array_size: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetInitialSubresource {
    pub subresource_ordinal: u32,
    pub row_pitch: u32,
    pub slice_pitch: u32,
    pub source: RenderTargetInitialSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetInitialSource {
    Inline {
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
        sha256: String,
    },
    UsedContentRange {
        content: UsedContentKey,
        source_byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedRenderTarget {
    SwapchainBackBuffer,
    OpaqueRenderTarget { attachment_ordinal: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewportDesc {
    pub top_left_x_f32_bits: u32,
    pub top_left_y_f32_bits: u32,
    pub width_f32_bits: u32,
    pub height_f32_bits: u32,
    pub min_depth_f32_bits: u32,
    pub max_depth_f32_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScissorDesc {
    pub left_i32: i32,
    pub top_i32: i32,
    pub right_i32: i32,
    pub bottom_i32: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOperation {
    Spawn,
    Activate,
    Deactivate,
    StopEmission,
    Destroy,
    Reparent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParentAnchor {
    Entity {
        #[serde(with = "decimal_u64")]
        entity_instance_id: u64,
        node_token: String,
    },
    World {
        world_transform_4x4_f32_bits: [u32; 16],
    },
    /// The engine's own scene-node anchor, recorded when the producer reported
    /// that no semantic join was obtained.
    ///
    /// Say it that way and not "no semantic owner packet claims the effect":
    /// the carrier verifies only that the identity is absent from the two
    /// censuses the provider receipts carry, which is a lookup against a known
    /// set, not a statement about ownership. Whether the producer genuinely
    /// attempted the join is a producer obligation (gate XMAP20-G1).
    ///
    /// `node_token` is the attachment name the engine SYNTHESISES for every
    /// particle descriptor (descriptor+0x10, built at 0x14208f200 from a fixed
    /// literal plus a process-global counter). It is unique within one process
    /// run and is NOT authored content, so it identifies the attachment inside
    /// a recording and must never be used as a cross-recording identity.
    ///
    /// `node_transform_4x4_f32_bits` is the scene node's own transform block at
    /// node+0x10, reached as `instance+0x40 - 0x10 + 0x10`. It is recorded as
    /// what the node holds. This variant deliberately makes NO claim that the
    /// transform is world-resolved; that is the difference from `World`, which
    /// additionally asserts equality with the particle packet matrix.
    SceneNode {
        node_token: String,
        node_transform_4x4_f32_bits: [u32; 16],
    },
}

pub const RAIL_CONSTRUCTION_ARROW_PRODUCER_RVA: u32 = 0x0159_B650;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RailConstructionArrowVisual {
    pub producer_rva: u32,
    pub province_lo: u32,
    pub province_hi: u32,
    pub edge_type_u8: u8,
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub entity_registration_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySubmission {
    pub pass_ordinal: u32,
    pub draw_order: u32,
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub resolved_entity_sha256: String,
    pub content_identity: EntityContentIdentity,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub state_flags: u32,
    pub skinning: Skinning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityContentIdentity {
    pub resolved_entity_asset: UsedContentKey,
    pub mesh: UsedContentKey,
    pub native_selection: EntityNativeSelection,
    pub native_selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityNativeSelection {
    pub entity_registration_token: String,
    pub entity_group_index: u32,
    pub mesh_draw_list_index: u32,
    pub entity_variant_u8: u8,
    pub mesh_draw_variant_u8: u8,
    pub pass_selector_i32: i32,
    pub alternate_list_u8: u8,
    pub bind_renderer_target_u8: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skinning {
    NotSkinned,
    Palette {
        palette_sha256: String,
        #[serde(with = "base64_bytes")]
        palette_3200_bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherMembership {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    pub strategic_region_id: u32,
    pub rain_light_active: bool,
    pub rain_heavy_active: bool,
    pub snow_active: bool,
    pub blizzard_active: bool,
    pub sandstorm_active: bool,
    pub arctic_water_active: bool,
    pub region_value_i64_fixed_1e5: i64,
    pub transition_expiry_game_hour_i32: i32,
    pub airborne_runtime_code: u8,
    pub selection_seed_words: Option<[u32; 2]>,
    pub visual_epoch: Option<WeatherVisualEpochStamp>,
    pub weather_effect_token: Option<String>,
    pub weather_position_source_content: UsedContentKey,
    pub ordered_variant_content: Vec<WeatherVariantContentJoin>,
    pub ordered_positions: Vec<WeatherPosition>,
    pub ordered_weather_instances: Vec<WeatherInstance>,
    pub visibility_updates: Vec<WeatherVisibilityPhase>,
    pub membership_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherVisualEpochStamp {
    #[serde(with = "decimal_u64")]
    pub ordinal: u64,
    pub game_hour_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherVariantContentJoin {
    pub authored_effect_index: u16,
    pub class: WeatherPositionClass,
    pub resolved_variant_token: String,
    pub primary: WeatherEntityContentJoin,
    pub fading: Option<WeatherEntityContentJoin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherEntityContentJoin {
    pub entity_token: String,
    pub ordered_content: Vec<UsedContentKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherVisibilityPhase {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    pub strategic_region_id: u32,
    pub view_distance_f32_bits: u32,
    pub camera_branch: WeatherCameraBranch,
    pub computed_fade_alpha_f32_bits: u32,
    pub computed_playback_rate_f32_bits: u32,
    pub ordered_entity_visibility: Vec<WeatherEntityVisibility>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherEntityVisibility {
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub role: WeatherEntityRole,
    pub post_update_active: bool,
    pub post_update_alpha_f32_bits: u32,
    pub post_update_playback_rate_f32_bits: u32,
    pub producer_wrote_active: bool,
    pub producer_wrote_alpha: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherEntityRole {
    Primary,
    Fading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherCameraBranch {
    NearHidden,
    NearFade,
    Full,
    FarFade,
    FarHidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherInstance {
    #[serde(with = "decimal_u64")]
    pub effect_instance_id: u64,
    pub position_ordinal: u16,
    pub class: WeatherPositionClass,
    pub resolved_variant_token: String,
    pub submitted_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderOutputRef {
    Particle {
        #[serde(with = "decimal_u64")]
        effect_instance_id: u64,
        bucket: u32,
        packet_ordinal: u32,
        draw_order: u32,
    },
    Entity {
        pass_ordinal: u32,
        #[serde(with = "decimal_u64")]
        entity_instance_id: u64,
        draw_order: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherPosition {
    pub position_ordinal: u16,
    pub file_source_ordinal: u32,
    pub source_line: u32,
    pub position_xyz_f32_bits: [u32; 3],
    pub class: WeatherPositionClass,
    pub resolution: WeatherResolution,
    pub selected_authored_effect_index: Option<u16>,
    pub primary_entity_instance_id: Option<String>,
    pub fading_entity_instance_id: Option<String>,
    pub ordered_effect_instance_ids: Vec<String>,
    pub resolved_variant_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherPositionClass {
    Big,
    Small,
}

pub fn weather_selected_variant_class(
    position_class: WeatherPositionClass,
    has_big_variants: bool,
) -> WeatherPositionClass {
    if position_class == WeatherPositionClass::Big && !has_big_variants {
        WeatherPositionClass::Small
    } else {
        position_class
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherResolution {
    Instantiated,
    ExplicitEmpty,
}

impl WeatherMembership {
    pub fn computed_membership_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.membership_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    pub fn seal_membership_sha256(&mut self) -> Result<(), String> {
        self.membership_sha256 = self.computed_membership_sha256()?;
        Ok(())
    }
}

/// Derives the per-region weather census the retired `RenderCheckpoint`
/// carried: the latest `WeatherMembership` row per strategic region across
/// every retained submit, in region order. This is the exact fold the
/// producer performed when sealing the old checkpoint field and the exact
/// fold the wire validator re-computed, so the derived census equals what
/// the carrier used to say on every sealed frame.
/// kills: carrier.production_frame.checkpoint
pub fn derive_weather_checkpoint_census(
    submits: &[RenderSubmit],
) -> Result<Vec<WeatherMembership>, String> {
    let mut weather_by_region = BTreeMap::new();
    for membership in submits.iter().flat_map(|submit| submit.weather_membership.iter()) {
        if weather_by_region
            .get(&membership.strategic_region_id)
            .is_some_and(|prior: &WeatherMembership| membership.sequence <= prior.sequence)
        {
            return Err("weather checkpoint source is repeated or non-monotonic by region".into());
        }
        weather_by_region.insert(membership.strategic_region_id, membership.clone());
    }
    Ok(weather_by_region.into_values().collect())
}

/// ownership or diplomatic state is never accepted as a substitute.
pub const OCCUPATION_SECONDARY_TARGET_BINDING: &str = "ProvinceSecondaryColorMap";
pub const OCCUPATION_SECONDARY_WIDTH: u32 = 2_816;
pub const OCCUPATION_SECONDARY_HEIGHT: u32 = 1_024;
pub const OCCUPATION_SECONDARY_BYTES_PER_TEXEL: u32 = 4;
pub const OCCUPATION_SECONDARY_ROW_PITCH: u32 =
    OCCUPATION_SECONDARY_WIDTH * OCCUPATION_SECONDARY_BYTES_PER_TEXEL;
pub const OCCUPATION_SECONDARY_BYTE_LENGTH: usize =
    OCCUPATION_SECONDARY_ROW_PITCH as usize * OCCUPATION_SECONDARY_HEIGHT as usize;
pub const OCCUPATION_SECONDARY_ALPHA: u8 = 76;
pub const OCCUPATION_SECONDARY_COVERAGE_ALPHAS: [u8; 5] = [0, 19, 38, 57, 76];
pub const OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA: &str =
    "hoi4.a729.occupation-secondary-upload.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupationSecondaryUploadReceipt {
    pub schema: String,
    pub executable_sha256: String,
    pub raster_worker_rva: u32,
    pub raster_worker_decompile_sha256: String,
    pub upload_target_rva: u32,
    pub upload_instructions_sha256: String,
    #[serde(with = "decimal_u64")]
    pub process_start_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub hook_epoch_nonce: u64,
    pub upload_order: u32,
    pub producer_game_hour_raw: i32,
    #[serde(with = "decimal_u64")]
    pub producer_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub producer_qpc_frequency_hz: u64,
    pub selected_present_sha256: String,
    pub payload_sha256: String,
    pub receipt_sha256: String,
}

impl OccupationSecondaryUploadReceipt {
    pub fn computed_receipt_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.receipt_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    pub fn validate_against(
        &self,
        packet: &PoliticalTexturePacket,
        selected_present: &SelectedPresentReceipt,
    ) -> Result<(), String> {
        if self.schema != OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA
            || self.executable_sha256 != A729_EXECUTABLE_SHA256
            || self.raster_worker_rva != 0x019c_8a10
            || self.raster_worker_decompile_sha256
                != "8915c1e7db348c9ed057dc2d1014ff472288c199f534ae7a9429625b895f996b"
            || self.upload_target_rva != SNOW_MUD_UPLOAD_TARGET_RVA
            || self.upload_instructions_sha256 != SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256
            || self.process_start_nonce == 0
            || self.hook_epoch_nonce == 0
            || self.upload_order != packet.upload_order
            || self.producer_qpc_ticks == 0
            || self.producer_qpc_frequency_hz == 0
            || self.producer_qpc_ticks > selected_present.selected_qpc_ticks
            || self.producer_game_hour_raw > selected_present.selected_game_hour_raw
            || self.selected_present_sha256 != selected_present.computed_sha256()?
            || self.payload_sha256 != packet.sha256
        {
            return Err("occupation secondary upload receipt identity/join mismatch".into());
        }
        for sha256 in [
            &self.executable_sha256,
            &self.raster_worker_decompile_sha256,
            &self.upload_instructions_sha256,
            &self.selected_present_sha256,
            &self.payload_sha256,
            &self.receipt_sha256,
        ] {
            check_sha256(sha256, None)?;
        }
        if self.receipt_sha256 != self.computed_receipt_sha256()? {
            return Err("occupation secondary upload receipt SHA-256 mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoliticalTexturePacket {
    pub kind: PoliticalTextureKind,
    pub target_binding_identity: String,
    pub upload_order: u32,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: PoliticalTextureFormat,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occupation_observation: Option<OccupationSecondaryUploadReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoliticalTextureKind {
    Primary,
    Secondary,
    GradientBorderChannel1,
    GradientBorderChannel2,
    GradientBorderChannel3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoliticalTextureFormat {
    #[serde(rename = "DXGI_FORMAT_B8G8R8A8_UNORM_SRGB")]
    B8g8r8a8UnormSrgb,
}

#[derive(Deserialize)]
struct SchemaProbe {
    schema: String,
    #[serde(default)]
    wire_schema: Option<String>,
}

const PRODUCTION_RENDER_WIRE_SCHEMA: &str = "hoi4.production-render-wire.v1";
const INLINE_BLOB_REFERENCE_FIELD: &str = "blob_sha256";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionRenderWire {
    schema: String,
    wire_schema: String,
    inline_blobs: Vec<InlineBlobWire>,
    frame: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineBlobWire {
    sha256: String,
    base64: String,
}

fn check_schema(schema: &str) -> Result<(), String> {
    if schema == INCOMPATIBLE_PRODUCTION_FRAME_CENSUS_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it still carries the retired per-frame RenderCheckpoint census, the display_submit_index hint that the retained submits already determine, and the per-frame RenderAuthority block whose session identity is sealed once in the recorder manifest instead"
        ));
    }
    if schema == INCOMPATIBLE_TILED_LIGHT_PAYLOAD_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: its selected submits still carry the retired tiled point-light payload carrier and live texture digests, which are forbidden product authority"
        ));
    }
    if schema == INCOMPATIBLE_PRESENTATION_CARRIER_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it carries always-empty entity-presentation and direct-visual producer vectors that no writer ever emitted instead of deriving presentation from the recorded entity/particle state"
        ));
    }
    if schema == INCOMPATIBLE_DERIVED_RUNTIME_CARRIER_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it carried the ordered runtime draw list and the C07 tree runtime carrier verbatim on the wire instead of deriving both from the recorded packets and selected submit"
        ));
    }
    if schema == INCOMPATIBLE_AUTHENTICATED_TEXTURE_SNAPSHOT_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve authenticated draw-time bytes for engine-generated textures that are neither direct effective-VFS payloads nor prior render-target outputs"
        ));
    }
    if schema == INCOMPATIBLE_RECORDED_INPUT_OBSERVATION_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: its recorded-input variant can carry only a digest and cannot preserve or replay-validate the immutable in-hook selected-Present observation chronology"
        ));
    }
    if schema == INCOMPATIBLE_SNOW_MUD_FADE_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: its SnowMudData packet is not joined to the exact selected-Present reflected SnowMudFade consumers"
        ));
    }
    if schema == INCOMPATIBLE_WEATHER_CARRIER_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve exact weather placement/content/visibility joins or the complete weather checkpoint hash fold"
        ));
    }
    if schema == INCOMPATIBLE_SNOW_MUD_PRESENT_LINK_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: its SnowMudData H observation does not seal the complete selected-Present candidate-set/cutoff receipt"
        ));
    }
    if schema == INCOMPATIBLE_CAMERA_EVENT_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve the authenticated raw C03 camera-event producer batches or their exact producer-to-target joins"
        ));
    }
    if schema == INCOMPATIBLE_S1_SHADOW_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve the exact selected-submit L02/L03 shadow projection, target identities, blur producer, and true-writer consumer chain"
        ));
    }
    if schema == INCOMPATIBLE_TREE_RUNTIME_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve the exact C07 tree visibility/model/LOD operation join or distinguish retained-subset from complete-pool coverage"
        ));
    }
    if schema == INCOMPATIBLE_SNOW_MUD_CARRIER_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: it cannot preserve the exact a729 SnowMudData CPU pre-upload observation"
        ));
    }
    if schema == INCOMPATIBLE_CAMERA_CARRIER_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: its envelope argument was mislabelled as interpolation f32 bits and it cannot preserve the authenticated C00/C01/C02 camera carrier"
        ));
    }
    if schema == INCOMPATIBLE_ENTITY_SELECTOR_SCHEMA {
        return Err(format!(
            "production render schema {schema} is incompatible with {SCHEMA}: exact conversion is unavailable because the older carrier does not preserve selector-native entity and particle authority"
        ));
    }
    if schema != SCHEMA {
        return Err(format!(
            "unsupported production render schema {schema}; expected {SCHEMA}"
        ));
    }
    Ok(())
}

fn deduplicate_inline_blob_references(
    value: &mut serde_json::Value,
    blobs: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                deduplicate_inline_blob_references(value, blobs)?;
            }
        }
        serde_json::Value::Object(object) => {
            if object.get("kind").and_then(serde_json::Value::as_str) == Some("inline") {
                let sha256 = object
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("inline buffer source has no SHA-256")?
                    .to_string();
                let encoded = object
                    .get("bytes")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("inline buffer source has no canonical base64 payload")?
                    .to_string();
                check_sha256(&sha256, None)?;
                let decoded = base64_bytes::decode(&encoded)?;
                if format!("{:x}", Sha256::digest(&decoded)) != sha256 {
                    return Err("inline buffer source bytes do not match its SHA-256".into());
                }
                match blobs.entry(sha256.clone()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(encoded);
                    }
                    std::collections::btree_map::Entry::Occupied(entry)
                        if entry.get() != &encoded =>
                    {
                        return Err(
                            "equal inline buffer SHA-256 values carry different bytes".into()
                        )
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {}
                }
                object.insert(
                    "bytes".into(),
                    serde_json::json!({ INLINE_BLOB_REFERENCE_FIELD: sha256 }),
                );
            }
            for value in object.values_mut() {
                deduplicate_inline_blob_references(value, blobs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn restore_inline_blob_references(
    value: &mut serde_json::Value,
    blobs: &BTreeMap<String, String>,
    used: &mut BTreeSet<String>,
) -> Result<(), String> {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                restore_inline_blob_references(value, blobs, used)?;
            }
        }
        serde_json::Value::Object(object) => {
            if object.get("kind").and_then(serde_json::Value::as_str) == Some("inline") {
                let reference = object
                    .get("bytes")
                    .and_then(serde_json::Value::as_object)
                    .ok_or("inline wire buffer source has no blob reference")?;
                if reference.len() != 1 {
                    return Err("inline wire buffer source blob reference is malformed".into());
                }
                let sha256 = reference
                    .get(INLINE_BLOB_REFERENCE_FIELD)
                    .and_then(serde_json::Value::as_str)
                    .ok_or("inline wire buffer source blob reference has no SHA-256")?
                    .to_string();
                if object.get("sha256").and_then(serde_json::Value::as_str) != Some(sha256.as_str())
                {
                    return Err(
                        "inline wire buffer source reference differs from source SHA-256".into(),
                    );
                }
                let encoded = blobs
                    .get(&sha256)
                    .ok_or("inline wire buffer source references an absent blob")?
                    .clone();
                used.insert(sha256);
                object.insert("bytes".into(), serde_json::Value::String(encoded));
            }
            for value in object.values_mut() {
                restore_inline_blob_references(value, blobs, used)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn submit_static_program_draws(submit: &RenderSubmit) -> impl Iterator<Item = &StaticMapDraw> {
    submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| match &operation.command {
            StaticMapCommand::Draw { draw } => Some(draw),
            StaticMapCommand::ClearColor { .. } | StaticMapCommand::ClearDepthStencil { .. } => {
                None
            }
        })
}

struct SideCarrierDraw<'a> {
    owner: &'static str,
    draw: &'a StaticMapDraw,
    execution: Option<RenderExecutionRef>,
}

struct FrameCarriers<'a> {
    m03: Option<&'a M03ArrowFrame>,
    snow_mud: Option<&'a SnowMudTexturePacket>,
}

impl ProductionRenderFrame {
    fn side_carrier_static_draws(&self) -> Vec<SideCarrierDraw<'_>> {
        let mut draws = Vec::new();
        if let Some(m03) = &self.m03_arrows {
            for draw in &m03.draws {
                draws.push(SideCarrierDraw {
                    owner: "M03",
                    draw: &draw.draw,
                    execution: Some(RenderExecutionRef::M03 {
                        output: M03ArrowDrawRef {
                            engine_draw_ordinal: draw.engine_draw_ordinal,
                        },
                    }),
                });
            }
        }
        if let Some(ui_pass) = &self.authenticated_ui_pass {
            for operation in &ui_pass.ui_pass_operations {
                let UiPassOperation::HudCommand { command } = operation;
                if let HudCommandExecution::ExactPipeline { draw } = &command.execution {
                    draws.push(SideCarrierDraw {
                        owner: "UI-pass HUD",
                        draw: &draw.exact_draw,
                        execution: None,
                    });
                }
            }
        }
        draws
    }

    pub fn decode_json(
        bytes: &[u8],
        admitted_effective_vfs_manifest_sha256: &str,
    ) -> Result<Self, String> {
        let probe: SchemaProbe = serde_json::from_slice(bytes)
            .map_err(|error| format!("production render JSON: {error}"))?;
        check_schema(&probe.schema)?;
        if probe.wire_schema.is_some() {
            let wire: ProductionRenderWire = serde_json::from_slice(bytes)
                .map_err(|error| format!("production render wire JSON: {error}"))?;
            if wire.schema != SCHEMA || wire.wire_schema != PRODUCTION_RENDER_WIRE_SCHEMA {
                return Err("production render wire identity mismatch".into());
            }
            let mut blobs = BTreeMap::new();
            for blob in wire.inline_blobs {
                check_sha256(&blob.sha256, None)?;
                let decoded = base64_bytes::decode(&blob.base64)?;
                if format!("{:x}", Sha256::digest(&decoded)) != blob.sha256
                    || blobs.insert(blob.sha256, blob.base64).is_some()
                {
                    return Err(
                        "production render wire blob table is hash-invalid or duplicated".into(),
                    );
                }
            }
            let mut frame = wire.frame;
            let mut used = BTreeSet::new();
            restore_inline_blob_references(&mut frame, &blobs, &mut used)?;
            if used.len() != blobs.len() {
                return Err("production render wire contains an unreferenced inline blob".into());
            }
            return Self::decode_value(&frame, admitted_effective_vfs_manifest_sha256);
        }
        let frame: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("production render JSON: {error}"))?;
        frame.validate(admitted_effective_vfs_manifest_sha256)?;
        Ok(frame)
    }

    pub fn decode_value(
        value: &serde_json::Value,
        admitted_effective_vfs_manifest_sha256: &str,
    ) -> Result<Self, String> {
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "production render JSON has no string schema".to_string())?;
        check_schema(schema)?;
        let frame: Self =
            Self::deserialize(value).map_err(|error| format!("production render JSON: {error}"))?;
        frame.validate(admitted_effective_vfs_manifest_sha256)?;
        Ok(frame)
    }

    pub fn encode_json(
        &self,
        admitted_effective_vfs_manifest_sha256: &str,
    ) -> Result<Vec<u8>, String> {
        self.validate(admitted_effective_vfs_manifest_sha256)?;
        let mut frame = serde_json::to_value(self).map_err(|error| error.to_string())?;
        let mut blobs = BTreeMap::new();
        deduplicate_inline_blob_references(&mut frame, &mut blobs)?;
        let wire = ProductionRenderWire {
            schema: SCHEMA.into(),
            wire_schema: PRODUCTION_RENDER_WIRE_SCHEMA.into(),
            inline_blobs: blobs
                .into_iter()
                .map(|(sha256, base64)| InlineBlobWire { sha256, base64 })
                .collect(),
            frame,
        };
        serde_json::to_vec(&wire).map_err(|error| error.to_string())
    }

    pub fn submit(&self, submit_index: u64) -> Option<&RenderSubmit> {
        if self.submits.len() != 1 || self.selected_present.selected_candidate_id != submit_index {
            return None;
        }
        self.submits.first()
    }

    pub fn retained_submit_identity(&self) -> Option<u64> {
        if self.submits.is_empty() {
            return None;
        }
        Some(self.selected_present.selected_candidate_id)
    }

    pub fn selected_presentation_qpc_ticks(&self) -> Option<u64> {
        if self.submits.is_empty() {
            return None;
        }
        Some(self.selected_present.selected_qpc_ticks)
    }

    pub fn validate(&self, admitted_effective_vfs_manifest_sha256: &str) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!(
                "unsupported production render schema {}",
                self.schema
            ));
        }
        check_sha256(admitted_effective_vfs_manifest_sha256, None)?;
        if let Some(observation) = &self.selected_present.recorded_input_observation {
            observation.validate()?;
            if observation.selected_present_qpc_ticks != self.selected_present.selected_qpc_ticks
                || observation.present_sample_completed_qpc_ticks
                    > self.selected_present.selection_cutoff_qpc_ticks
                || observation.selected_present_receipt_sha256
                    != self.selected_present.computed_sha256()?
            {
                return Err(
                    "production render recorded-input observation crosses selected-Present cutoff"
                        .into(),
                );
            }
        }
        if self.first_sequence > self.last_sequence {
            return Err("production render sequence range is inverted".into());
        }
        if self.submits.len() != 1 {
            return Err("production render must retain exactly one selected Present".into());
        }
        let selected = &self.selected_present;
        if selected.candidate_count == 0
            || selected.selected_ordinal.checked_add(1) != Some(selected.candidate_count)
            || compact_update_index_from_raw(selected.selected_game_hour_raw)
                != Some(self.update_index)
            || selected.selection_cutoff_qpc_ticks == 0
            || selected.selected_qpc_ticks > selected.selection_cutoff_qpc_ticks
        {
            return Err(
                "selected-Present receipt differs from the sole retained submit/hour/cutoff".into(),
            );
        }
        check_sha256(&selected.candidate_set_sha256, None)?;
        let mut last_global = None;
        if self.presentation_qpc_frequency_hz == 0 {
            return Err("presentation QPC frequency is zero".into());
        }
        let has_snow_mud_source = self
            .submits
            .iter()
            .flat_map(submit_static_program_draws)
            .chain(
                self.side_carrier_static_draws()
                    .into_iter()
                    .map(|carried| carried.draw),
            )
            .flat_map(|draw| draw.ordered_shader_resources.iter())
            .any(|binding| {
                matches!(
                    &binding.source,
                    StaticShaderResourceSource::SnowMudTexture { .. }
                )
            });
        match (&self.snow_mud_texture, has_snow_mud_source) {
            (Some(snow_mud), true) => snow_mud.validate_against(self)?,
            (None, false) => {}
            (Some(_), false) => {
                return Err("SnowMudData H packet has no selected-submit source edge".into())
            }
            (None, true) => {
                return Err("selected-submit SnowMudTexture source has no H packet".into())
            }
        }
        let carriers = FrameCarriers {
            m03: self.m03_arrows.as_ref(),
            snow_mud: self.snow_mud_texture.as_ref(),
        };
        for submit in &self.submits {
            if last_global.is_some_and(|v| submit.global_order <= v) {
                return Err("global_order is not strictly increasing".into());
            }
            last_global = Some(submit.global_order);
            submit.validate(
                self.first_sequence,
                self.last_sequence,
                admitted_effective_vfs_manifest_sha256,
                &carriers,
            )?;
        }
        let side_carrier_draws = self.side_carrier_static_draws();
        if !side_carrier_draws.is_empty() {
            let display_submit = self
                .submit(selected.selected_candidate_id)
                .ok_or("selected Present has no retained submit for its overlay carriers")?;
            let scheduled = display_submit
                .ordered_render_operations
                .iter()
                .collect::<BTreeSet<_>>();
            for carried in side_carrier_draws {
                if carried
                    .execution
                    .as_ref()
                    .is_some_and(|execution| scheduled.contains(execution))
                {
                    continue;
                }
                validate_static_draw_named_payloads(
                    carried.draw,
                    &NamedPayloadScope {
                        submit: display_submit,
                        order: PreparedTargetOrder::Unordered,
                        snow_mud: SnowMudResolution::RequirePacket(self.snow_mud_texture.as_ref()),
                        owner: carried.owner,
                    },
                )?;
            }
        }
        for packet in self
            .submits
            .iter()
            .flat_map(|submit| submit.political_textures.iter())
        {
            match (
                packet.kind,
                packet.target_binding_identity.as_str(),
                packet.occupation_observation.as_ref(),
            ) {
                (
                    PoliticalTextureKind::Secondary,
                    OCCUPATION_SECONDARY_TARGET_BINDING,
                    Some(receipt),
                ) => receipt.validate_against(packet, selected)?,
                (PoliticalTextureKind::Secondary, OCCUPATION_SECONDARY_TARGET_BINDING, None) => {
                    return Err(
                        "occupation secondary packet lacks an authenticated CPU upload receipt"
                            .into(),
                    )
                }
                (_, _, Some(_)) => {
                    return Err(
                        "non-occupation political packet carries an occupation upload receipt"
                            .into(),
                    )
                }
                (_, _, None) => {}
            }
        }
        if let Some(ui_pass) = &self.authenticated_ui_pass {
            let observation = self
                .selected_present
                .recorded_input_observation
                .as_ref()
                .ok_or(
                    "selected-Present UI pass lacks authenticated recorded input variant receipt",
                )?;
            ui_pass.validate()?;
            if ui_pass.update_index != self.update_index
                || ui_pass.first_sequence != self.first_sequence
                || ui_pass.last_sequence != self.last_sequence
                || ui_pass.selected_present_receipt_sha256
                    != self.selected_present.computed_sha256()?
                || ui_pass.recorded_input_receipt_sha256 != observation.observation_receipt_sha256
                || observation.selected_present_receipt_sha256
                    != ui_pass.selected_present_receipt_sha256
            {
                return Err(
                    "selected-Present UI-pass frontier/selector/input receipt does not match production frame"
                        .into(),
                );
            }
            validate_render_target_vfs_authority(
                &ui_pass.target,
                admitted_effective_vfs_manifest_sha256,
                "selected-Present UI pass",
            )?;
            if let Some(ActiveMapmodeState {
                provenance: MapmodeProvenance::EffectiveVfs { content },
                ..
            }) = &ui_pass.active_mapmode_state
            {
                if content.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
                {
                    return Err("active mapmode VFS authority differs from production frame".into());
                }
            }
            for operation in &ui_pass.ui_pass_operations {
                match operation {
                    UiPassOperation::HudCommand { command } => {
                        if command
                            .ordered_effective_vfs_used_content
                            .iter()
                            .chain(std::iter::once(&command.identity.selected_gui_definition))
                            .any(|content| {
                                content.effective_vfs_manifest_sha256
                                    != admitted_effective_vfs_manifest_sha256
                            })
                        {
                            return Err(
                                "UI-pass HUD content authority differs from production frame"
                                    .into(),
                            );
                        }
                        if let HudCommandExecution::ExactPipeline { draw } = &command.execution {
                            validate_static_draw_vfs_authority(
                                &draw.exact_draw,
                                admitted_effective_vfs_manifest_sha256,
                                "UI-pass HUD",
                            )?;
                        }
                    }
                }
            }
        }
        if let Some(m03) = &self.m03_arrows {
            for content in &m03.used_content {
                if content.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
                {
                    return Err("M03 content authority differs from production frame".into());
                }
            }
            m03.validate()?;
            if m03.update_index != self.update_index
                || m03.first_sequence != self.first_sequence
                || m03.last_sequence != self.last_sequence
            {
                return Err("M03 update/sequence frontier does not match production frame".into());
            }
            for draw in &m03.draws {
                validate_static_draw_vfs_authority(
                    &draw.draw,
                    admitted_effective_vfs_manifest_sha256,
                    "M03",
                )?;
            }
            let display_submit = self
                .submits
                .first()
                .ok_or("M03 carrier has no authenticated display submit")?;
            let expected = m03.ordered_draws.iter().cloned().collect::<Vec<_>>();
            let actual = display_submit
                .ordered_render_operations
                .iter()
                .filter_map(|operation| match operation {
                    RenderExecutionRef::M03 { output } => Some(output.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if actual != expected {
                return Err(
                    "display submit M03 operations do not exactly cover carrier order".into(),
                );
            }
        } else if self.submits.iter().any(|submit| {
            submit
                .ordered_render_operations
                .iter()
                .any(|operation| matches!(operation, RenderExecutionRef::M03 { .. }))
        }) {
            return Err("unified render order references absent M03 carrier".into());
        }
        if let Some(rail) = &self.rail_supply_arrows {
            rail.validate()?;
            if rail.update_index != self.update_index
                || rail.first_sequence != self.first_sequence
                || rail.last_sequence != self.last_sequence
            {
                return Err(
                    "rail supply-arrow update/sequence frontier does not match production frame"
                        .into(),
                );
            }
            for content in rail.used_content.iter().chain(&rail.draw_used_content) {
                if content.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
                {
                    return Err(
                        "rail supply-arrow content authority differs from production frame".into(),
                    );
                }
            }
            let display_submit = self
                .submits
                .first()
                .ok_or("rail supply-arrow carrier has no authenticated display submit")?;
            rail.validate_against_static_programs(&display_submit.ordered_static_map_programs)?;
            let ordered_static = display_submit
                .ordered_render_operations
                .iter()
                .filter_map(|reference| match reference {
                    RenderExecutionRef::Static { operation_ordinal } => Some(*operation_ordinal),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut previous_position = None;
            for reference in &rail.ordered_draws {
                let position = ordered_static
                    .iter()
                    .position(|ordinal| *ordinal == reference.operation_ordinal)
                    .ok_or(
                        "rail supply-arrow draw lacks an authenticated static interleave position",
                    )?;
                if previous_position.is_some_and(|previous| previous >= position) {
                    return Err(
                        "rail supply-arrow draw order disagrees with authenticated static interleave"
                            .into(),
                    );
                }
                previous_position = Some(position);
            }
        }
        if let Some(o04) = &self.o04_cities {
            o04.validate()?;
            if o04.update_index != self.update_index
                || o04.first_sequence != self.first_sequence
                || o04.last_sequence != self.last_sequence
            {
                return Err("O04 update/sequence frontier does not match production frame".into());
            }
            let display_submit = self
                .submits
                .first()
                .ok_or("O04 carrier has no authenticated display submit")?;
            let referenced = o04.ordered_outputs.iter().cloned().collect::<BTreeSet<_>>();
            let actual_order = display_submit
                .derive_ordered_draws()?
                .iter()
                .filter(|output| referenced.contains(*output))
                .cloned()
                .collect::<Vec<_>>();
            if actual_order != o04.ordered_outputs {
                return Err("O04 output order differs from authenticated display submit".into());
            }
            for city in &o04.cities {
                if city.entity_asset.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
                    || city.mesh.effective_vfs_manifest_sha256
                        != admitted_effective_vfs_manifest_sha256
                {
                    return Err("O04 content authority differs from production frame".into());
                }
                let entity_instance_id = match city.referenced_outputs.first() {
                    Some(RenderOutputRef::Entity {
                        entity_instance_id, ..
                    }) => *entity_instance_id,
                    _ => return Err("O04 city has no entity output identity".into()),
                };
                let complete_display_outputs = display_submit
                    .derive_ordered_draws()?
                    .iter()
                    .filter(|output| {
                        matches!(
                            output,
                            RenderOutputRef::Entity {
                                entity_instance_id: candidate,
                                ..
                            } if *candidate == entity_instance_id
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if complete_display_outputs != city.referenced_outputs {
                    return Err(
                        "O04 city output refs are not the complete display-submit entity cover"
                            .into(),
                    );
                }
                for output in &city.referenced_outputs {
                    let entity = match output {
                        RenderOutputRef::Entity {
                            pass_ordinal,
                            entity_instance_id,
                            draw_order,
                        } => display_submit.entities.iter().find(|entity| {
                            entity.pass_ordinal == *pass_ordinal
                                && entity.entity_instance_id == *entity_instance_id
                                && entity.draw_order == *draw_order
                        }),
                        RenderOutputRef::Particle { .. } => None,
                    }
                    .ok_or("O04 city references an absent entity submission")?;
                    if entity.content_identity.resolved_entity_asset != city.entity_asset
                        || entity.content_identity.mesh != city.mesh
                        || entity.world_transform_4x4_f32_bits != city.world_transform_4x4_f32_bits
                    {
                        return Err(
                            "O04 semantic identity/transform differs from entity submission".into(),
                        );
                    }
                }
            }
        }
        if let Some(o05) = &self.o05_buildings {
            o05.validate()?;
            if o05.update_index != self.update_index
                || o05.first_sequence != self.first_sequence
                || o05.last_sequence != self.last_sequence
                || o05.map_buildings_source.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
            {
                return Err(
                    "O05 update/sequence/content authority differs from production frame".into(),
                );
            }
            let display_submit = self
                .submits
                .first()
                .ok_or("O05 carrier has no authenticated display submit")?;
            let referenced = o05.ordered_outputs.iter().cloned().collect::<BTreeSet<_>>();
            let actual_order = display_submit
                .derive_ordered_draws()?
                .iter()
                .filter(|output| referenced.contains(*output))
                .cloned()
                .collect::<Vec<_>>();
            if actual_order != o05.ordered_outputs {
                return Err("O05 output order differs from authenticated display submit".into());
            }
            for building in &o05.records {
                let Some(first_output) = building.referenced_outputs.first() else {
                    continue;
                };
                let entity_instance_id = match first_output {
                    RenderOutputRef::Entity {
                        entity_instance_id, ..
                    } => *entity_instance_id,
                    RenderOutputRef::Particle { .. } => unreachable!("validated O05 output"),
                };
                let complete_display_outputs = display_submit
                    .derive_ordered_draws()?
                    .iter()
                    .filter(|output| {
                        matches!(
                            output,
                            RenderOutputRef::Entity {
                                entity_instance_id: candidate,
                                ..
                            } if *candidate == entity_instance_id
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if complete_display_outputs != building.referenced_outputs {
                    return Err(
                        "O05 output refs are not the complete display-submit entity cover".into(),
                    );
                }
                let entity_asset = building.entity_asset.as_ref().expect("validated O05 asset");
                let mesh = building.mesh.as_ref().expect("validated O05 mesh");
                if entity_asset.effective_vfs_manifest_sha256
                    != admitted_effective_vfs_manifest_sha256
                    || mesh.effective_vfs_manifest_sha256
                        != admitted_effective_vfs_manifest_sha256
                {
                    return Err(
                        "O05 building content authority differs from production frame".into(),
                    );
                }
                for output in &building.referenced_outputs {
                    let entity = match output {
                        RenderOutputRef::Entity {
                            pass_ordinal,
                            entity_instance_id,
                            draw_order,
                        } => display_submit.entities.iter().find(|entity| {
                            entity.pass_ordinal == *pass_ordinal
                                && entity.entity_instance_id == *entity_instance_id
                                && entity.draw_order == *draw_order
                        }),
                        RenderOutputRef::Particle { .. } => None,
                    }
                    .ok_or("O05 building references an absent entity submission")?;
                    if entity.content_identity.resolved_entity_asset != *entity_asset
                        || entity.content_identity.mesh != *mesh
                        || entity.world_transform_4x4_f32_bits
                            != building
                                .world_transform_4x4_f32_bits
                                .expect("validated O05 transform")
                    {
                        return Err(
                            "O05 semantic asset/mesh/transform differs from entity submission"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

impl RenderSubmit {
    /// Derives the exact runtime draw order from the recorded particle and
    /// entity packets alone. The capture-side producer law (collector
    /// promotion) builds the cross-family draw sequence by sorting every
    /// particle/entity output reference on its `draw_order` key and refusing
    /// anything that is not a complete contiguous `0..n` sequence, so the
    /// carried list was always a pure function of the packets this submit
    /// already records. Deriving it here keeps that law fail-closed on the
    /// receive side instead of spending wire bytes on a verbatim copy.
    pub fn derive_ordered_draws(&self) -> Result<Vec<RenderOutputRef>, String> {
        let mut ordered = self
            .particles
            .iter()
            .map(|packet| RenderOutputRef::Particle {
                effect_instance_id: packet.effect_instance_id,
                bucket: packet.bucket,
                packet_ordinal: packet.packet_ordinal,
                draw_order: packet.draw_order,
            })
            .chain(self.entities.iter().map(|entity| {
                RenderOutputRef::Entity {
                    pass_ordinal: entity.pass_ordinal,
                    entity_instance_id: entity.entity_instance_id,
                    draw_order: entity.draw_order,
                }
            }))
            .collect::<Vec<_>>();
        ordered.sort_by_key(|draw| match draw {
            RenderOutputRef::Particle { draw_order, .. }
            | RenderOutputRef::Entity { draw_order, .. } => *draw_order,
        });
        if ordered.iter().enumerate().any(|(expected, draw)| {
            let actual = match draw {
                RenderOutputRef::Particle { draw_order, .. }
                | RenderOutputRef::Entity { draw_order, .. } => *draw_order,
            };
            usize::try_from(actual).ok() != Some(expected)
        }) {
            return Err(
                "runtime draw order is not a complete contiguous cross-family sequence".into(),
            );
        }
        Ok(ordered)
    }

    fn validate(
        &self,
        first_sequence: u64,
        last_sequence: u64,
        authority_manifest_sha256: &str,
        carriers: &FrameCarriers<'_>,
    ) -> Result<(), String> {
        if let Some(camera) = &self.camera_carrier {
            camera.validate()?;
        }
        check_sha256(&self.particle_packet_set_sha256, None)?;
        let mut authenticated_texture_snapshots = BTreeMap::new();
        let mut prior_authenticated_texture_snapshot = None;
        for packet in &self.authenticated_texture_snapshots {
            packet.validate()?;
            if prior_authenticated_texture_snapshot
                .is_some_and(|prior: &str| packet.snapshot_sha256.as_str() <= prior)
            {
                return Err(
                    "authenticated texture snapshot packets are not in strict digest order".into(),
                );
            }
            prior_authenticated_texture_snapshot = Some(packet.snapshot_sha256.as_str());
            if authenticated_texture_snapshots
                .insert(packet.snapshot_sha256.as_str(), packet)
                .is_some()
            {
                return Err(
                    "render submit repeats an authenticated texture snapshot packet".into(),
                );
            }
        }
        validate_static_map_programs(
            &self.ordered_static_map_programs,
            authority_manifest_sha256,
        )?;
        if let Some(s1_shadow) = &self.s1_shadow {
            s1_shadow.validate_against_submit(self)?;
        }
        validate_optional_render_carrier_joins(self)?;
        let mut particle_keys = BTreeSet::new();
        let mut particle_outputs = BTreeSet::new();
        for packet in &self.particles {
            if !particle_keys.insert((packet.bucket, packet.packet_ordinal, packet.draw_order)) {
                return Err("duplicate particle packet order key".into());
            }
            if packet.canonical_packet_112_bytes.len() != 112 {
                return Err("particle canonical packet is not 112 bytes".into());
            }
            if packet.canonical_packet_112_bytes[0x48..0x50] != [0; 8] {
                return Err("particle canonical packet retained process pointer".into());
            }
            check_sha256(
                &packet.canonical_packet_sha256,
                Some(&packet.canonical_packet_112_bytes),
            )?;
            let canonical = &packet.canonical_packet_112_bytes;
            for (word, bits) in canonical[..0x40]
                .chunks_exact(4)
                .zip(packet.packet_matrix_4x4_f32_bits)
            {
                if word != bits.to_le_bytes() {
                    return Err(
                        "particle canonical packet matrix disagrees with typed matrix".into(),
                    );
                }
            }
            for (offset, bits, field) in [
                (0x40, packet.live_count, "live count"),
                (0x50, packet.record_stride, "record stride"),
                (0x54, packet.max_count, "max count"),
                (0x58, packet.elapsed_f32_bits, "elapsed bits"),
                (0x60, packet.packet_ordinal, "packet ordinal"),
            ] {
                if canonical[offset..offset + 4] != bits.to_le_bytes() {
                    return Err(format!(
                        "particle canonical packet disagrees with typed {field}"
                    ));
                }
            }
            if canonical[0x5c..0x5e] != packet.flags_u16.to_le_bytes() {
                return Err("particle canonical packet disagrees with typed flags".into());
            }
            if packet.record_stride != 48 {
                return Err("particle record stride is not 48".into());
            }
            let expected = usize::try_from(packet.live_count)
                .ok()
                .and_then(|count| count.checked_mul(48))
                .ok_or_else(|| "particle instance byte count overflow".to_string())?;
            if packet.ordered_gpu_instance_records.len() != expected
                || packet.live_count > packet.max_count
            {
                return Err("particle live count/instance bytes/max count mismatch".into());
            }
            check_sha256(
                &packet.gpu_instance_records_sha256,
                Some(&packet.ordered_gpu_instance_records),
            )?;
            check_sha256(&packet.resolved_definition_sha256, None)?;
            packet.content_identity.validate()?;
            check_sha256(&packet.content_identity_sha256, None)?;
            if packet.content_identity.computed_sha256()? != packet.content_identity_sha256 {
                return Err("particle content identity SHA-256 mismatch".into());
            }
            if packet.resolved_definition_sha256
                != packet.content_identity.definition_content.payload_sha256
            {
                return Err(
                    "particle resolved hash does not match admitted definition payload hash".into(),
                );
            }
            if packet.packet_ordinal != packet.content_identity.subsystem_selector_u32 {
                return Err("particle packet ordinal does not match subsystem selector".into());
            }
            packet.shader_program.validate()?;
            validate_texture_bindings(&packet.ordered_texture_bindings)?;
            if packet.shader_program.effect_token != packet.content_identity.engine_effect_token {
                return Err("particle engine effect token does not match program receipt".into());
            }
            let selected_texture = packet
                .ordered_texture_bindings
                .iter()
                .find(|binding| binding.content == packet.content_identity.texture_content)
                .ok_or_else(|| {
                    "particle selected texture winner is absent from exact bindings".to_string()
                })?;
            let expected_half_width = (0.5f32 / selected_texture.resource.width as f32).to_bits();
            let expected_half_height = (0.5f32 / selected_texture.resource.height as f32).to_bits();
            if packet.content_identity.atlas_half_pixel_width_f32_bits != expected_half_width
                || packet.content_identity.atlas_half_pixel_height_f32_bits != expected_half_height
            {
                return Err(
                    "particle atlas half-pixel bits do not match selected texture dimensions"
                        .into(),
                );
            }
            packet.pipeline.validate()?;
            packet.geometry.validate(&packet.pipeline, true)?;
            validate_constant_buffers(&packet.ordered_constant_buffers)?;
            packet.output_state.validate()?;
            validate_runtime_draw_vfs_authority(
                &[
                    &packet.content_identity.definition_content,
                    &packet.content_identity.texture_content,
                ],
                &packet.shader_program,
                &packet.ordered_texture_bindings,
                &packet.geometry,
                &packet.output_state,
                authority_manifest_sha256,
                "particle",
            )?;
            if packet.atlas_rows == 0 || packet.atlas_columns == 0 {
                return Err("particle atlas dimensions are zero".into());
            }
            if packet.atlas_columns != packet.content_identity.atlas_columns_u32
                || packet.atlas_rows != packet.content_identity.atlas_rows_u32
            {
                return Err("particle atlas dimensions do not match authored identity".into());
            }
            particle_outputs.insert(RenderOutputRef::Particle {
                effect_instance_id: packet.effect_instance_id,
                bucket: packet.bucket,
                packet_ordinal: packet.packet_ordinal,
                draw_order: packet.draw_order,
            });
        }

        let mut entity_keys = BTreeSet::new();
        let mut entity_outputs = BTreeSet::new();
        for entity in &self.entities {
            if !entity_keys.insert((entity.pass_ordinal, entity.entity_instance_id)) {
                return Err("duplicate entity (pass, instance) key".into());
            }
            check_sha256(&entity.resolved_entity_sha256, None)?;
            entity.content_identity.validate()?;
            if entity.resolved_entity_sha256
                != entity.content_identity.resolved_entity_asset.payload_sha256
            {
                return Err(
                    "entity resolved hash does not match admitted asset payload hash".into(),
                );
            }
            if entity.state_flags
                != entity
                    .content_identity
                    .native_selection
                    .packed_state_flags()
            {
                return Err(
                    "entity state_flags do not equal the selector-native packed tuple".into(),
                );
            }
            entity.shader_program.validate()?;
            validate_texture_bindings(&entity.ordered_texture_bindings)?;
            entity.pipeline.validate()?;
            entity.geometry.validate(&entity.pipeline, false)?;
            validate_constant_buffers(&entity.ordered_constant_buffers)?;
            entity.output_state.validate()?;
            validate_runtime_draw_vfs_authority(
                &[
                    &entity.content_identity.resolved_entity_asset,
                    &entity.content_identity.mesh,
                ],
                &entity.shader_program,
                &entity.ordered_texture_bindings,
                &entity.geometry,
                &entity.output_state,
                authority_manifest_sha256,
                "entity",
            )?;
            if let Skinning::Palette {
                palette_sha256,
                palette_3200_bytes,
            } = &entity.skinning
            {
                if palette_3200_bytes.len() != 3200 {
                    return Err("skinning palette is not 3200 bytes".into());
                }
                check_sha256(palette_sha256, Some(palette_3200_bytes))?;
            }
            entity_outputs.insert(RenderOutputRef::Entity {
                pass_ordinal: entity.pass_ordinal,
                entity_instance_id: entity.entity_instance_id,
                draw_order: entity.draw_order,
            });
        }

        let mut prior_rail_key = None;
        let mut rail_entity_ids = BTreeSet::new();
        for arrow in &self.rail_construction_arrows {
            let key = (arrow.province_lo, arrow.province_hi);
            if arrow.producer_rva != RAIL_CONSTRUCTION_ARROW_PRODUCER_RVA
                || arrow.province_lo == 0
                || arrow.province_lo >= arrow.province_hi
                || !matches!(arrow.edge_type_u8, 2 | 3)
                || arrow.entity_instance_id == 0
                || arrow.entity_registration_token.is_empty()
                || prior_rail_key.is_some_and(|prior| prior >= key)
                || !rail_entity_ids.insert(arrow.entity_instance_id)
            {
                return Err(
                    "rail construction-arrow visual rows are invalid, reordered, or duplicated"
                        .into(),
                );
            }
            let joined = self.entities.iter().filter(|entity| {
                entity.entity_instance_id == arrow.entity_instance_id
                    && entity
                        .content_identity
                        .native_selection
                        .entity_registration_token
                        == arrow.entity_registration_token
            });
            if joined.count() == 0 {
                return Err(
                    "rail construction-arrow visual has no exact entity submission/content join"
                        .into(),
                );
            }
            prior_rail_key = Some(key);
        }

        let all_outputs: BTreeSet<_> = particle_outputs
            .iter()
            .chain(entity_outputs.iter())
            .cloned()
            .collect();
        let ordered_draws = self.derive_ordered_draws()?;
        if ordered_draws.len() != all_outputs.len()
            || ordered_draws.iter().any(|output| !all_outputs.contains(output))
        {
            return Err(
                "derived runtime draw order does not cover every particle/entity draw exactly once"
                    .into(),
            );
        }
        validate_unified_render_operations(self, carriers)?;

        let mut previous_weather_sequence = None;
        for weather in &self.weather_membership {
            if previous_weather_sequence.is_some_and(|value| weather.sequence <= value) {
                return Err("weather membership sequence is repeated or non-monotonic".into());
            }
            previous_weather_sequence = Some(weather.sequence);
            validate_weather_membership(
                weather,
                first_sequence,
                last_sequence,
                authority_manifest_sha256,
                &particle_outputs,
                &entity_outputs,
                &ordered_draws,
            )?;
        }
        let mut texture_orders = BTreeSet::new();
        for texture in &self.political_textures {
            if !texture_orders.insert(texture.upload_order) {
                return Err("duplicate political texture upload order".into());
            }
            let expected = usize::try_from(texture.row_pitch)
                .ok()
                .and_then(|pitch| pitch.checked_mul(texture.height as usize))
                .ok_or_else(|| "political texture byte count overflow".to_string())?;
            if texture.upload_order == 0 || texture.target_binding_identity.trim().is_empty() {
                return Err("political texture has no exact target binding identity".into());
            }
            if texture.width == 0 || texture.height == 0 || texture.bytes.len() != expected {
                return Err("political texture descriptor/byte count mismatch".into());
            }
            check_sha256(&texture.sha256, Some(&texture.bytes))?;
            let consumed_by_selected_submit = self
                .ordered_static_map_programs
                .iter()
                .flat_map(|program| program.ordered_operations.iter())
                .filter_map(|operation| match &operation.command {
                    StaticMapCommand::Draw { draw } => Some(draw),
                    StaticMapCommand::ClearColor { .. }
                    | StaticMapCommand::ClearDepthStencil { .. } => None,
                })
                .flat_map(|draw| draw.ordered_shader_resources.iter())
                .any(|binding| {
                    matches!(
                        &binding.source,
                        StaticShaderResourceSource::PoliticalTexture {
                            texture_kind,
                            target_binding_identity,
                            upload_order,
                            payload_sha256,
                            ..
                        } if *texture_kind == texture.kind
                            && target_binding_identity == &texture.target_binding_identity
                            && *upload_order == texture.upload_order
                            && payload_sha256 == &texture.sha256
                    )
                });
            if !consumed_by_selected_submit {
                return Err(
                    "political texture packet has no exact selected-submit static shader-resource consumer"
                        .into(),
                );
            }
            if texture.kind == PoliticalTextureKind::Secondary
                && texture.target_binding_identity == OCCUPATION_SECONDARY_TARGET_BINDING
            {
                validate_occupation_secondary_packet(self, texture)?;
            }
        }
        Ok(())
    }
}

fn validate_occupation_secondary_packet(
    submit: &RenderSubmit,
    texture: &PoliticalTexturePacket,
) -> Result<(), String> {
    let alpha_law_is_exact = texture
        .bytes
        .chunks_exact(OCCUPATION_SECONDARY_BYTES_PER_TEXEL as usize)
        .all(|texel| OCCUPATION_SECONDARY_COVERAGE_ALPHAS.contains(&texel[3]))
        && texture
            .bytes
            .chunks_exact(OCCUPATION_SECONDARY_BYTES_PER_TEXEL as usize)
            .any(|texel| texel[3] != 0);
    if texture.width != OCCUPATION_SECONDARY_WIDTH
        || texture.height != OCCUPATION_SECONDARY_HEIGHT
        || texture.row_pitch != OCCUPATION_SECONDARY_ROW_PITCH
        || texture.bytes.len() != OCCUPATION_SECONDARY_BYTE_LENGTH
        || texture.format != PoliticalTextureFormat::B8g8r8a8UnormSrgb
        || !alpha_law_is_exact
    {
        return Err(
            "occupation secondary packet violates the exact descriptor/nontransparent alpha law"
                .into(),
        );
    }
    let mut terrain_t6 = 0u32;
    let mut water_t3 = 0u32;
    for binding in submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| match &operation.command {
            StaticMapCommand::Draw { draw } => Some(draw),
            StaticMapCommand::ClearColor { .. } | StaticMapCommand::ClearDepthStencil { .. } => {
                None
            }
        })
        .flat_map(|draw| draw.ordered_shader_resources.iter())
    {
        let StaticShaderResourceSource::PoliticalTexture {
            texture_kind,
            target_binding_identity,
            upload_order,
            payload_sha256,
            ..
        } = &binding.source
        else {
            continue;
        };
        if *texture_kind != texture.kind
            || target_binding_identity != &texture.target_binding_identity
            || *upload_order != texture.upload_order
            || payload_sha256 != &texture.sha256
        {
            continue;
        }
        if binding.stage != ShaderStage::Pixel
            || binding.semantic_token != OCCUPATION_SECONDARY_TARGET_BINDING
        {
            return Err(
                "occupation secondary consumer differs from its reflected pixel semantic".into(),
            );
        }
        match binding.slot {
            6 => {
                terrain_t6 = terrain_t6
                    .checked_add(1)
                    .ok_or("occupation secondary terrain consumer count overflow")?
            }
            3 => {
                water_t3 = water_t3
                    .checked_add(1)
                    .ok_or("occupation secondary water consumer count overflow")?
            }
            _ => return Err("occupation secondary consumer is outside terrain-t6/water-t3".into()),
        }
    }
    if terrain_t6 == 0 || water_t3 == 0 {
        return Err(
            "occupation secondary packet lacks complete terrain-t6/water-t3 consumers".into(),
        );
    }
    Ok(())
}

fn validate_optional_render_carrier_joins(submit: &RenderSubmit) -> Result<(), String> {
    if let (Some(camera), Some(shadow)) = (&submit.camera_carrier, &submit.s1_shadow) {
        if !shadow.is_inactive() && shadow.camera_y_f32_bits != Some(camera.current_eye_f32_bits[1])
        {
            return Err("C02 camera current-eye Y differs from active S1 camera Y".into());
        }
    }
    // The retired C07/S1 tree-shadow join lives inside
    // S1ShadowCarrier::validate_against_submit (see s1_shadow.rs): every
    // carried S1 carrier validates against its submit wherever it is admitted,
    // so the derived-C07 join cannot be bypassed by skipping this helper.
    Ok(())
}

fn validate_weather_membership(
    weather: &WeatherMembership,
    first_sequence: u64,
    last_sequence: u64,
    authority_manifest_sha256: &str,
    particle_outputs: &BTreeSet<RenderOutputRef>,
    entity_outputs: &BTreeSet<RenderOutputRef>,
    ordered_draws: &[RenderOutputRef],
) -> Result<(), String> {
    check_sequence(weather.sequence, first_sequence, last_sequence, "weather")?;
    if weather.membership_sha256 != weather.computed_membership_sha256()? {
        return Err("weather membership SHA-256 mismatch".into());
    }
    weather.weather_position_source_content.validate()?;
    if weather
        .weather_position_source_content
        .normalized_game_relative_path
        != "map/weatherpositions.txt"
        || weather
            .weather_position_source_content
            .effective_vfs_manifest_sha256
            != authority_manifest_sha256
    {
        return Err("weather position source has the wrong path or VFS authority".into());
    }

    let expected_token = match weather.airborne_runtime_code {
        0 => None,
        2 => Some("rain_light"),
        3 => Some("rain_heavy"),
        4 => Some("snow"),
        5 => Some("blizzard"),
        7 => Some("sandstorm"),
        _ => return Err("weather membership contains an unknown airborne code".into()),
    };
    let expected_code = if weather.rain_light_active {
        2
    } else if weather.rain_heavy_active {
        3
    } else if weather.snow_active {
        4
    } else if weather.blizzard_active {
        5
    } else if weather.sandstorm_active {
        7
    } else {
        0
    };
    if weather.airborne_runtime_code != expected_code {
        return Err("weather airborne code disagrees with frozen flag priority".into());
    }
    if weather.weather_effect_token.as_deref() != expected_token {
        return Err("weather effect token disagrees with airborne code".into());
    }
    if (weather.airborne_runtime_code == 0) != weather.selection_seed_words.is_none() {
        return Err("weather selection seed presence disagrees with the airborne code".into());
    }
    if weather.visual_epoch.is_some_and(|epoch| epoch.ordinal == 0) {
        return Err("weather visual epoch ordinal is zero".into());
    }

    let mut variant_keys = BTreeSet::new();
    for variant in &weather.ordered_variant_content {
        let key = (
            variant.class,
            variant.authored_effect_index,
            variant.resolved_variant_token.as_str(),
        );
        if variant.resolved_variant_token.trim().is_empty() || !variant_keys.insert(key) {
            return Err("weather variant content key is blank or duplicated".into());
        }
        for entity in std::iter::once(&variant.primary).chain(variant.fading.iter()) {
            if entity.entity_token.trim().is_empty() || entity.ordered_content.is_empty() {
                return Err("weather entity content closure is blank".into());
            }
            let mut content_paths = BTreeSet::new();
            for content in &entity.ordered_content {
                content.validate()?;
                if content.effective_vfs_manifest_sha256 != authority_manifest_sha256
                    || !content_paths.insert(content.normalized_game_relative_path.as_str())
                {
                    return Err(
                        "weather entity content is foreign or repeats a logical path".into(),
                    );
                }
            }
        }
    }
    if weather.airborne_runtime_code == 0 && !weather.ordered_variant_content.is_empty() {
        return Err("code-zero weather carries authored variant content".into());
    }

    let mut expected_entities = Vec::new();
    let mut expected_effects = Vec::new();
    let mut prior_source_line = None;
    for (expected_ordinal, position) in weather.ordered_positions.iter().enumerate() {
        if usize::from(position.position_ordinal) != expected_ordinal
            || position.source_line == 0
            || prior_source_line.is_some_and(|line| position.source_line <= line)
            || position
                .position_xyz_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("weather position source/order/XYZ identity is invalid".into());
        }
        prior_source_line = Some(position.source_line);
        match position.resolution {
            WeatherResolution::ExplicitEmpty => {
                if position.selected_authored_effect_index.is_some()
                    || position.primary_entity_instance_id.is_some()
                    || position.fading_entity_instance_id.is_some()
                    || !position.ordered_effect_instance_ids.is_empty()
                    || position.resolved_variant_token.is_some()
                {
                    return Err("explicit-empty weather position contains live identity".into());
                }
            }
            WeatherResolution::Instantiated => {
                let selected_index = position
                    .selected_authored_effect_index
                    .ok_or("instantiated weather position has no authored index")?;
                let variant_token = position
                    .resolved_variant_token
                    .as_deref()
                    .ok_or("instantiated weather position has no variant token")?;
                let primary = position
                    .primary_entity_instance_id
                    .as_deref()
                    .ok_or("instantiated weather position has no primary entity")?;
                check_decimal_u64(primary, "weather primary entity")?;
                let primary_id = primary
                    .parse::<u64>()
                    .map_err(|_| "weather primary entity is not u64")?;
                if primary_id == 0 {
                    return Err("weather primary entity id is zero".into());
                }
                expected_entities.push((primary_id, WeatherEntityRole::Primary));
                if let Some(fading) = position.fading_entity_instance_id.as_deref() {
                    check_decimal_u64(fading, "weather fading entity")?;
                    let fading_id = fading
                        .parse::<u64>()
                        .map_err(|_| "weather fading entity is not u64")?;
                    if fading_id == 0 {
                        return Err("weather fading entity id is zero".into());
                    }
                    expected_entities.push((fading_id, WeatherEntityRole::Fading));
                }
                let expected_variant_class = weather_selected_variant_class(
                    position.class,
                    weather
                        .ordered_variant_content
                        .iter()
                        .any(|variant| variant.class == WeatherPositionClass::Big),
                );
                if weather
                    .ordered_variant_content
                    .iter()
                    .filter(|variant| {
                        variant.class == expected_variant_class
                            && variant.authored_effect_index == selected_index
                            && variant.resolved_variant_token == variant_token
                    })
                    .count()
                    != 1
                {
                    return Err("weather position has no unique content variant join".into());
                }
                for effect in &position.ordered_effect_instance_ids {
                    check_decimal_u64(effect, "weather effect instance")?;
                    let effect = effect
                        .parse::<u64>()
                        .map_err(|_| "weather effect instance is not u64")?;
                    if effect == 0 {
                        return Err("weather effect instance id is zero".into());
                    }
                    expected_effects.push((
                        effect,
                        position.position_ordinal,
                        position.class,
                        variant_token,
                    ));
                }
            }
        }
    }
    if weather.airborne_runtime_code == 0
        && (weather
            .ordered_positions
            .iter()
            .any(|position| position.resolution != WeatherResolution::ExplicitEmpty)
            || !weather.ordered_weather_instances.is_empty()
            || !weather.visibility_updates.is_empty())
    {
        return Err("code-zero weather is not explicit-empty".into());
    }
    if weather.airborne_runtime_code != 0 && weather.visibility_updates.is_empty() {
        return Err("active weather has no admitted visibility/playback observation".into());
    }

    if weather.ordered_weather_instances.len() != expected_effects.len() {
        return Err("weather instance count disagrees with position effect membership".into());
    }
    let mut instance_keys = BTreeSet::new();
    for (instance, expected) in weather
        .ordered_weather_instances
        .iter()
        .zip(expected_effects.iter())
    {
        if !instance_keys.insert((instance.effect_instance_id, instance.position_ordinal))
            || instance.effect_instance_id != expected.0
            || instance.position_ordinal != expected.1
            || instance.class != expected.2
            || instance.resolved_variant_token != expected.3
        {
            return Err("weather instance identity/order/output join is incomplete".into());
        }
        validate_output_refs(
            &instance.submitted_outputs,
            particle_outputs,
            entity_outputs,
            "weather instance",
        )?;
        let complete_effect_outputs = ordered_draws
            .iter()
            .filter(|output| {
                matches!(
                    output,
                    RenderOutputRef::Particle {
                        effect_instance_id,
                        ..
                    } if *effect_instance_id == instance.effect_instance_id
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        if instance.submitted_outputs != complete_effect_outputs {
            return Err(
                "weather instance outputs are not the complete ordered particle cover".into(),
            );
        }
    }

    let mut previous_sequence = None;
    let mut previous_update = None;
    let mut prior_entity_state: BTreeMap<u64, (bool, u32)> = BTreeMap::new();
    for visibility in &weather.visibility_updates {
        check_sequence(
            visibility.sequence,
            first_sequence,
            last_sequence,
            "weather visibility",
        )?;
        if visibility.strategic_region_id != weather.strategic_region_id
            || visibility.update_index < weather.update_index
            || visibility.sequence <= weather.sequence
            || previous_sequence.is_some_and(|value| visibility.sequence <= value)
            || previous_update.is_some_and(|value| visibility.update_index < value)
        {
            return Err("weather visibility order/region/update identity is invalid".into());
        }
        previous_sequence = Some(visibility.sequence);
        previous_update = Some(visibility.update_index);
        let (branch, alpha_bits, playback_bits) =
            weather_visibility_law(visibility.view_distance_f32_bits)?;
        if visibility.camera_branch != branch
            || visibility.computed_fade_alpha_f32_bits != alpha_bits
            || visibility.computed_playback_rate_f32_bits != playback_bits
            || visibility.ordered_entity_visibility.len() != expected_entities.len()
        {
            return Err("weather visibility differs from the frozen camera law".into());
        }
        for (entity, expected) in visibility
            .ordered_entity_visibility
            .iter()
            .zip(expected_entities.iter())
        {
            let (expected_active_write, expected_alpha_write) = match (entity.role, branch) {
                (WeatherEntityRole::Primary, WeatherCameraBranch::FarFade) => (true, true),
                (WeatherEntityRole::Primary, WeatherCameraBranch::FarHidden) => (true, false),
                (WeatherEntityRole::Primary, _) => (false, false),
                (WeatherEntityRole::Fading, WeatherCameraBranch::NearHidden)
                | (WeatherEntityRole::Fading, WeatherCameraBranch::FarHidden) => (true, false),
                (WeatherEntityRole::Fading, _) => (true, true),
            };
            if (entity.entity_instance_id, entity.role) != *expected
                || entity.producer_wrote_active != expected_active_write
                || entity.producer_wrote_alpha != expected_alpha_write
                || entity.post_update_playback_rate_f32_bits != playback_bits
                || (expected_active_write && entity.post_update_active != (alpha_bits != 0))
                || (expected_alpha_write && entity.post_update_alpha_f32_bits != alpha_bits)
            {
                return Err("weather entity visibility/write-mask law mismatch".into());
            }
            if let Some(prior) = prior_entity_state.get(&entity.entity_instance_id) {
                if (!expected_active_write && prior.0 != entity.post_update_active)
                    || (!expected_alpha_write && prior.1 != entity.post_update_alpha_f32_bits)
                {
                    return Err(
                        "weather producer changed an entity field in a no-write branch".into(),
                    );
                }
            }
            prior_entity_state.insert(
                entity.entity_instance_id,
                (entity.post_update_active, entity.post_update_alpha_f32_bits),
            );
        }
    }
    Ok(())
}

pub fn weather_visibility_law(
    view_distance_f32_bits: u32,
) -> Result<(WeatherCameraBranch, u32, u32), String> {
    let distance = f32::from_bits(view_distance_f32_bits);
    if !distance.is_finite() || distance < 0.0 {
        return Err("weather view distance is non-finite or negative".into());
    }
    let (branch, alpha): (WeatherCameraBranch, f32) = if distance < 358.0 {
        (WeatherCameraBranch::NearHidden, 0.0)
    } else if distance < 578.0 {
        (WeatherCameraBranch::NearFade, (distance - 358.0) / 220.0)
    } else if distance <= 1100.0 {
        (WeatherCameraBranch::Full, 1.0)
    } else if distance <= 1500.0 {
        (WeatherCameraBranch::FarFade, (1500.0 - distance) / 400.0)
    } else {
        (WeatherCameraBranch::FarHidden, 0.0)
    };
    let playback: f32 = if distance <= 300.0 {
        1.0
    } else if distance < 500.0 {
        ((500.0 - distance) / 200.0) * 0.85 + 0.15
    } else {
        0.15
    };
    Ok((branch, alpha.to_bits(), playback.to_bits()))
}

impl StaticMapPassProgram {
    pub fn seal_payload_hashes(&mut self) -> Result<(), String> {
        for operation in &mut self.ordered_operations {
            if let StaticMapCommand::Draw { draw } = &mut operation.command {
                draw.payload_sha256 = draw.computed_payload_sha256()?;
            }
            operation.payload_sha256 = operation.computed_payload_sha256()?;
        }
        self.payload_sha256 = self.computed_payload_sha256()?;
        Ok(())
    }

    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }
}

impl StaticMapOperation {
    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }
}

impl StaticMapDraw {
    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }

    pub fn mapname_d80_state(&self) -> Result<MapnameD80State, String> {
        let stamped = self
            .ordered_constant_buffers
            .iter()
            .filter(|binding| binding.semantic_layout == MAPNAME_TRANSP_OFFSET_SEMANTIC)
            .collect::<Vec<_>>();
        if self.shader_program.effect_token != "mapname" {
            if !stamped.is_empty() {
                return Err(
                    "mapname Transp_OffsetX semantic is bound to a non-mapname static draw".into(),
                );
            }
            return Ok(MapnameD80State::Missing);
        }
        if stamped.is_empty() {
            return Ok(MapnameD80State::LegacyUnstamped);
        }

        const MAPNAME_STAGES: u8 = 0b0000_0011;
        const CB_BYTES: usize = 32;
        let mut covered_stages = 0u8;
        let mut admitted: Option<[u8; CB_BYTES]> = None;
        for binding in &stamped {
            if binding.stage_mask_u8 == 0 || binding.stage_mask_u8 & !MAPNAME_STAGES != 0 {
                return Err("mapname Transp_OffsetX CB has non-VS/PS stage coverage".into());
            }
            if binding.slot != 1
                || binding.byte_offset != 0
                || binding.byte_count as usize != CB_BYTES
                || binding.raw_bytes.len() != CB_BYTES
            {
                return Err("mapname Transp_OffsetX CB is not the exact CB1 32-byte range".into());
            }
            check_sha256(&binding.sha256, Some(&binding.raw_bytes))?;
            let bytes: [u8; CB_BYTES] = binding
                .raw_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "mapname Transp_OffsetX CB byte conversion failed")?;
            if admitted.as_ref().is_some_and(|prior| prior != &bytes) {
                return Err("mapname VS/PS Transp_OffsetX observations disagree".into());
            }
            admitted.get_or_insert(bytes);
            covered_stages |= binding.stage_mask_u8;
        }
        if covered_stages != MAPNAME_STAGES {
            return Err("mapname Transp_OffsetX CB lacks complete VS+PS coverage".into());
        }
        if self.ordered_constant_buffers.iter().any(|binding| {
            binding.slot == 1
                && binding.stage_mask_u8 & MAPNAME_STAGES != 0
                && binding.semantic_layout != MAPNAME_TRANSP_OFFSET_SEMANTIC
        }) {
            return Err(
                "authenticated mapname CB1 is shadowed by an unstamped VS/PS binding".into(),
            );
        }
        let bytes = admitted.expect("stamped mapname bindings are non-empty");
        let alpha = f32::from_le_bytes(bytes[0..4].try_into().expect("four alpha bytes"));
        let wrap = f32::from_le_bytes(bytes[4..8].try_into().expect("four wrap bytes"));
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) || !wrap.is_finite() {
            return Err("mapname Transp_OffsetX alpha/wrap values are invalid".into());
        }
        Ok(MapnameD80State::Authenticated)
    }
}

impl StaticMapPassProgram {
    pub fn mapname_d80_state(&self) -> Result<MapnameD80State, String> {
        self.ordered_operations
            .iter()
            .try_fold(
                MapnameD80State::Missing,
                |state, operation| match &operation.command {
                    StaticMapCommand::Draw { draw } => state.merge(draw.mapname_d80_state()?),
                    StaticMapCommand::ClearColor { .. }
                    | StaticMapCommand::ClearDepthStencil { .. } => Ok(state),
                },
            )
    }
}

pub fn canonical_json_sha256(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_static_map_programs(
    programs: &[StaticMapPassProgram],
    authority_manifest_sha256: &str,
) -> Result<(), String> {
    if programs.is_empty() {
        return Ok(());
    }
    // kills: carrier.render_submit.ordered_scene_constant_buffers — the retired submit-level
    // scene constant-buffer carrier never had a producer edge (the only constructor wrote an
    // empty vec), so no sealed frame ever carried scene-CB bytes here; program content is
    // still validated on its own merits and the scene-CB authority join is re-introduced by
    // the producer lane that seals real constant-buffer bytes.

    let mut previous_program = None;
    let mut previous_operation = None;
    let mut produced_targets = BTreeMap::<PreparedRenderTarget, u32>::new();
    let mut swapchain_output_present = false;
    let mut prepared_target_present = false;
    for program in programs {
        if program.pass_token.trim().is_empty() {
            return Err("static map program has a blank opaque pass identity".into());
        }
        if previous_program.is_some_and(|previous| program.program_ordinal <= previous) {
            return Err("static map program ordinals are not strictly increasing".into());
        }
        previous_program = Some(program.program_ordinal);
        if program.ordered_operations.is_empty() {
            return Err("static map program contains no authenticated operation".into());
        }
        let mut content_keys = BTreeSet::new();
        for key in &program.ordered_content_keys {
            if key.trim().is_empty() || !content_keys.insert(key) {
                return Err("static map program content-key order is blank or duplicated".into());
            }
        }
        check_sha256(&program.payload_sha256, None)?;
        if program.computed_payload_sha256()? != program.payload_sha256 {
            return Err("static map program payload SHA-256 mismatch".into());
        }

        for operation in &program.ordered_operations {
            if previous_operation.is_some_and(|previous| operation.operation_ordinal <= previous) {
                return Err("static map operation ordinals are not globally increasing".into());
            }
            previous_operation = Some(operation.operation_ordinal);
            check_sha256(&operation.payload_sha256, None)?;
            if operation.computed_payload_sha256()? != operation.payload_sha256 {
                return Err("static map operation payload SHA-256 mismatch".into());
            }

            match &operation.command {
                StaticMapCommand::ClearColor {
                    target,
                    rgba_f32_bits,
                } => {
                    validate_static_target(target, false)?;
                    validate_render_target_vfs_authority(
                        target,
                        authority_manifest_sha256,
                        "static map",
                    )?;
                    if rgba_f32_bits
                        .iter()
                        .any(|bits| !f32::from_bits(*bits).is_finite())
                    {
                        return Err("static color clear carries a non-finite component".into());
                    }
                    produced_targets
                        .insert(target.prepared_target.clone(), operation.operation_ordinal);
                }
                StaticMapCommand::ClearDepthStencil {
                    target,
                    clear_depth,
                    clear_stencil,
                    depth_f32_bits,
                    ..
                } => {
                    if !clear_depth && !clear_stencil {
                        return Err("depth/stencil clear selects neither plane".into());
                    }
                    validate_static_target(target, true)?;
                    validate_render_target_vfs_authority(
                        target,
                        authority_manifest_sha256,
                        "static map",
                    )?;
                    let depth = f32::from_bits(*depth_f32_bits);
                    if *clear_depth && (!depth.is_finite() || !(0.0..=1.0).contains(&depth)) {
                        return Err("static depth clear is non-finite or outside [0,1]".into());
                    }
                    produced_targets
                        .insert(target.prepared_target.clone(), operation.operation_ordinal);
                }
                StaticMapCommand::Draw { draw } => {
                    validate_static_map_draw(draw)?;
                    validate_static_draw_vfs_authority(
                        draw,
                        authority_manifest_sha256,
                        "static map",
                    )?;
                    if draw
                        .output_state
                        .ordered_color_targets
                        .iter()
                        .any(|target| {
                            target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                        })
                        && draw.ordered_shader_resources.iter().any(|binding| {
                            matches!(
                                &binding.source,
                                StaticShaderResourceSource::PreparedTarget { .. }
                            )
                        })
                    {
                        prepared_target_present = true;
                    }
                    swapchain_output_present |=
                        draw.output_state
                            .ordered_color_targets
                            .iter()
                            .any(|target| {
                                target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                            });
                    for target in draw
                        .output_state
                        .ordered_color_targets
                        .iter()
                        .chain(draw.output_state.depth_target.iter())
                    {
                        produced_targets
                            .insert(target.prepared_target.clone(), operation.operation_ordinal);
                    }
                }
            }
        }
    }
    if swapchain_output_present && !prepared_target_present {
        return Err("swapchain output has no authenticated draw from a prepared target".into());
    }
    Ok(())
}

fn validate_unified_render_operations(
    submit: &RenderSubmit,
    carriers: &FrameCarriers<'_>,
) -> Result<(), String> {
    let static_operations = submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .map(|operation| (operation.operation_ordinal, operation))
        .collect::<BTreeMap<_, _>>();
    if !static_operations.is_empty() && submit.ordered_render_operations.is_empty() {
        return Err("static map programs have no unified render-operation order".into());
    }
    if submit.ordered_render_operations.is_empty() {
        return Ok(());
    }
    let m03_count = submit
        .ordered_render_operations
        .iter()
        .filter(|operation| matches!(operation, RenderExecutionRef::M03 { .. }))
        .count();
    let ordered_draws = submit.derive_ordered_draws()?;
    if submit.ordered_render_operations.len()
        != static_operations.len() + ordered_draws.len() + m03_count
    {
        return Err(
            "unified render-operation order does not cover every static/runtime operation".into(),
        );
    }

    let mut seen = BTreeSet::new();
    let mut previous_static_ordinal = None;
    let mut runtime_index = 0usize;
    let mut last_writer = BTreeMap::<PreparedRenderTarget, RenderExecutionRef>::new();
    for execution in &submit.ordered_render_operations {
        if !seen.insert(execution.clone()) {
            return Err("unified render-operation order contains a duplicate".into());
        }
        match execution {
            RenderExecutionRef::Static { operation_ordinal } => {
                if previous_static_ordinal.is_some_and(|previous| *operation_ordinal <= previous) {
                    return Err("unified render order changes static operation order".into());
                }
                previous_static_ordinal = Some(*operation_ordinal);
                let operation = static_operations
                    .get(operation_ordinal)
                    .ok_or("unified render order references an unknown static operation")?;
                if let StaticMapCommand::Draw { draw } = &operation.command {
                    validate_static_draw_named_payloads(
                        draw,
                        &NamedPayloadScope {
                            submit,
                            order: PreparedTargetOrder::Ordered {
                                current: execution,
                                last_writer: &last_writer,
                            },
                            snow_mud: SnowMudResolution::SealedConsumerSet,
                            owner: "static map",
                        },
                    )?;
                }
                for target in static_operation_outputs(operation) {
                    last_writer.insert(target.prepared_target.clone(), execution.clone());
                }
            }
            RenderExecutionRef::Runtime { output } => {
                if ordered_draws.get(runtime_index) != Some(output) {
                    return Err(
                        "unified render order changes authenticated runtime draw order".into(),
                    );
                }
                runtime_index += 1;
                let output_state = runtime_output_state(submit, output)
                    .ok_or("unified render order references an unknown runtime draw")?;
                record_output_state_writer(&mut last_writer, output_state, execution);
            }
            RenderExecutionRef::M03 { output } => {
                if let Some(draw) = carriers.m03.and_then(|m03| m03.draw(output)) {
                    validate_static_draw_named_payloads(
                        &draw.draw,
                        &NamedPayloadScope {
                            submit,
                            order: PreparedTargetOrder::Ordered {
                                current: execution,
                                last_writer: &last_writer,
                            },
                            snow_mud: SnowMudResolution::RequirePacket(carriers.snow_mud),
                            owner: "M03",
                        },
                    )?;
                    record_output_state_writer(
                        &mut last_writer,
                        &draw.draw.output_state,
                        execution,
                    );
                }
            }
        }
    }
    if runtime_index != ordered_draws.len()
        || seen
            .iter()
            .filter(|value| matches!(value, RenderExecutionRef::Static { .. }))
            .count()
            != static_operations.len()
    {
        return Err("unified render-operation order has incomplete one-to-one coverage".into());
    }
    let referenced_snapshots = submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| &program.ordered_operations)
        .filter_map(|operation| match &operation.command {
            StaticMapCommand::Draw { draw } => Some(draw),
            StaticMapCommand::ClearColor { .. } | StaticMapCommand::ClearDepthStencil { .. } => {
                None
            }
        })
        .flat_map(|draw| &draw.ordered_shader_resources)
        .filter_map(|binding| match &binding.source {
            StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256, ..
            } => Some(snapshot_sha256.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let carried_snapshots = submit
        .authenticated_texture_snapshots
        .iter()
        .map(|packet| packet.snapshot_sha256.as_str())
        .collect::<BTreeSet<_>>();
    if referenced_snapshots != carried_snapshots {
        return Err(
            "authenticated texture snapshot packet table has missing or unreferenced rows".into(),
        );
    }
    Ok(())
}

enum PreparedTargetOrder<'a> {
    Ordered {
        current: &'a RenderExecutionRef,
        last_writer: &'a BTreeMap<PreparedRenderTarget, RenderExecutionRef>,
    },
    Unordered,
}

enum SnowMudResolution<'a> {
    SealedConsumerSet,
    RequirePacket(Option<&'a SnowMudTexturePacket>),
}

struct NamedPayloadScope<'a> {
    submit: &'a RenderSubmit,
    order: PreparedTargetOrder<'a>,
    snow_mud: SnowMudResolution<'a>,
    owner: &'a str,
}

fn validate_static_draw_named_payloads(
    draw: &StaticMapDraw,
    scope: &NamedPayloadScope<'_>,
) -> Result<(), String> {
    let submit = scope.submit;
    let owner = scope.owner;
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::PreparedTarget {
                target, producer, ..
            } => {
                let PreparedTargetOrder::Ordered {
                    current,
                    last_writer,
                } = &scope.order
                else {
                    return Err(format!(
                        "{owner} draw binds a prepared-target SRV with no authenticated producer position in the render-operation order"
                    ));
                };
                if producer == *current || last_writer.get(target) != Some(producer) {
                    return Err(
                        "prepared-target SRV does not name its exact preceding last writer".into(),
                    );
                }
            }
            StaticShaderResourceSource::PoliticalTexture {
                texture_kind,
                target_binding_identity,
                upload_order,
                payload_sha256,
                resource,
                ..
            } => {
                let mut matches = submit.political_textures.iter().filter(|packet| {
                    packet.kind == *texture_kind
                        && packet.target_binding_identity == *target_binding_identity
                        && packet.upload_order == *upload_order
                });
                let packet = matches.next().ok_or_else(|| {
                    format!("{owner} draw references an absent political texture packet")
                })?;
                if matches.next().is_some()
                    || packet.sha256 != *payload_sha256
                    || resource.width != packet.width
                    || resource.height != packet.height
                    || resource.dxgi_format_u32 != 91
                    || resource.sample_count != 1
                {
                    return Err("political texture source identity/descriptor differs from selected submit payload".into());
                }
            }
            StaticShaderResourceSource::SnowMudTexture {
                producer_ordinal,
                payload_sha256,
                resource,
                ..
            } => {
                if *producer_ordinal == 0
                    || resource.width != SNOW_MUD_TEXTURE_WIDTH
                    || resource.height != SNOW_MUD_TEXTURE_HEIGHT
                    || resource.dxgi_format_u32 != 87
                    || resource.sample_count != 1
                {
                    return Err("SnowMudTexture source identity/descriptor is incomplete".into());
                }
                check_sha256(payload_sha256, None)?;
                if let SnowMudResolution::RequirePacket(packet) = &scope.snow_mud {
                    let packet = packet.ok_or_else(|| {
                        format!("{owner} draw references an absent SnowMudData H packet")
                    })?;
                    if packet.producer_ordinal != *producer_ordinal
                        || packet.payload_sha256 != *payload_sha256
                    {
                        return Err(format!(
                            "{owner} draw SnowMudTexture source is not joined to the selected-Present H packet"
                        ));
                    }
                }
            }
            // The semantic point-light census behind these textures is
            // owner-unproven (ledger row lighting.point_light_lifecycle,
            // portable path none), so no lawful payload exists for ANY draw:
            // unavailable is never rendered as measured empty.
            StaticShaderResourceSource::TiledLightIndex { .. } => {
                return Err(format!(
                    "{owner} draw binds a tiled point-light index texture, but no lawful payload exists until the owner proves the semantic point-light census"
                ));
            }
            StaticShaderResourceSource::TiledLightData { .. } => {
                return Err(format!(
                    "{owner} draw binds a tiled point-light data texture, but no lawful payload exists until the owner proves the semantic point-light census"
                ));
            }
            StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256,
                view,
            } => {
                check_sha256(snapshot_sha256, None)?;
                let packet = submit
                    .authenticated_texture_snapshots
                    .iter()
                    .find(|packet| packet.snapshot_sha256 == *snapshot_sha256)
                    .ok_or_else(|| {
                        format!(
                            "{owner} draw references an absent authenticated texture snapshot packet"
                        )
                    })?;
                view.validate()?;
                view.validate_against(&packet.resource)?;
            }
            StaticShaderResourceSource::UsedContent { .. } => {}
        }
    }
    Ok(())
}

fn record_output_state_writer(
    last_writer: &mut BTreeMap<PreparedRenderTarget, RenderExecutionRef>,
    output_state: &RenderOutputState,
    execution: &RenderExecutionRef,
) {
    for target in output_state
        .ordered_color_targets
        .iter()
        .chain(output_state.depth_target.iter())
    {
        last_writer.insert(target.prepared_target.clone(), execution.clone());
    }
}

fn static_operation_outputs(operation: &StaticMapOperation) -> Vec<&RenderTargetDesc> {
    match &operation.command {
        StaticMapCommand::ClearColor { target, .. }
        | StaticMapCommand::ClearDepthStencil { target, .. } => vec![target],
        StaticMapCommand::Draw { draw } => draw
            .output_state
            .ordered_color_targets
            .iter()
            .chain(draw.output_state.depth_target.iter())
            .collect(),
    }
}

fn runtime_output_state<'a>(
    submit: &'a RenderSubmit,
    output: &RenderOutputRef,
) -> Option<&'a RenderOutputState> {
    match output {
        RenderOutputRef::Particle {
            effect_instance_id,
            bucket,
            packet_ordinal,
            draw_order,
        } => submit
            .particles
            .iter()
            .find(|packet| {
                packet.effect_instance_id == *effect_instance_id
                    && packet.bucket == *bucket
                    && packet.packet_ordinal == *packet_ordinal
                    && packet.draw_order == *draw_order
            })
            .map(|packet| &packet.output_state),
        RenderOutputRef::Entity {
            pass_ordinal,
            entity_instance_id,
            draw_order,
        } => submit
            .entities
            .iter()
            .find(|entity| {
                entity.pass_ordinal == *pass_ordinal
                    && entity.entity_instance_id == *entity_instance_id
                    && entity.draw_order == *draw_order
            })
            .map(|entity| &entity.output_state),
    }
}

fn validate_static_map_draw(draw: &StaticMapDraw) -> Result<(), String> {
    if draw.draw_token.trim().is_empty() {
        return Err("static map draw token is blank".into());
    }
    check_sha256(&draw.payload_sha256, None)?;
    if draw.computed_payload_sha256()? != draw.payload_sha256 {
        return Err("static map draw payload SHA-256 mismatch".into());
    }
    draw.shader_program.validate()?;
    draw.pipeline.validate()?;
    validate_constant_buffers(&draw.ordered_constant_buffers)?;
    draw.mapname_d80_state()?;
    draw.output_state.validate()?;
    validate_static_output_state(&draw.output_state)?;

    let mut resource_slots = BTreeSet::new();
    for binding in &draw.ordered_shader_resources {
        if binding.semantic_token.trim().is_empty()
            || !resource_slots.insert((binding.stage, binding.slot))
        {
            return Err("static shader resource is unnamed or duplicates a stage/slot".into());
        }
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content,
                resource,
                view,
            } => {
                content.validate()?;
                resource.validate()?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256,
                view,
            } => {
                check_sha256(snapshot_sha256, None)?;
                view.validate()?;
            }
            StaticShaderResourceSource::PreparedTarget {
                target,
                resource,
                view,
                ..
            } => {
                target.validate()?;
                validate_prepared_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::PoliticalTexture {
                target_binding_identity,
                payload_sha256,
                resource,
                view,
                ..
            } => {
                if target_binding_identity.trim().is_empty() {
                    return Err("political texture source has no target binding identity".into());
                }
                check_sha256(payload_sha256, None)?;
                validate_submit_payload_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::SnowMudTexture {
                producer_ordinal,
                payload_sha256,
                resource,
                view,
            } => {
                if *producer_ordinal == 0 {
                    return Err("SnowMudTexture source has zero producer ordinal".into());
                }
                check_sha256(payload_sha256, None)?;
                validate_submit_payload_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::TiledLightIndex { resource, view, .. }
            | StaticShaderResourceSource::TiledLightData { resource, view, .. } => {
                validate_submit_payload_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
        }
    }
    draw.geometry.validate(&draw.pipeline)?;
    Ok(())
}

fn validate_static_draw_vfs_authority(
    draw: &StaticMapDraw,
    expected_manifest_sha256: &str,
    owner: &str,
) -> Result<(), String> {
    let check = |content: &UsedContentKey| {
        if content.effective_vfs_manifest_sha256 == expected_manifest_sha256 {
            Ok(())
        } else {
            Err(format!(
                "{owner} draw content authority differs from production frame"
            ))
        }
    };
    check(&draw.shader_program.source)?;
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content, resource, ..
            } => {
                check(content)?;
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
            StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256,
                view,
            } => {
                check_sha256(snapshot_sha256, None)?;
                view.validate()?;
            }
            StaticShaderResourceSource::PreparedTarget { resource, .. } => {
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
            StaticShaderResourceSource::SnowMudTexture { resource, .. }
            | StaticShaderResourceSource::PoliticalTexture { resource, .. }
            | StaticShaderResourceSource::TiledLightIndex { resource, .. }
            | StaticShaderResourceSource::TiledLightData { resource, .. } => {
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for binding in &geometry.ordered_vertex_buffers {
            validate_buffer_source_vfs_authority(&binding.source, &check)?;
        }
        if let Some(binding) = &geometry.index_buffer {
            validate_buffer_source_vfs_authority(&binding.source, &check)?;
        }
    }
    for target in draw
        .output_state
        .ordered_color_targets
        .iter()
        .chain(draw.output_state.depth_target.iter())
    {
        if let Some(content) = &target.persistent_content {
            check(content)?;
        }
        if let Some(exact) = &target.exact_resource {
            for initial in &exact.ordered_initial_subresources {
                if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source
                {
                    check(content)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_render_target_vfs_authority(
    target: &RenderTargetDesc,
    expected_manifest_sha256: &str,
    owner: &str,
) -> Result<(), String> {
    let check = |content: &UsedContentKey| {
        if content.effective_vfs_manifest_sha256 == expected_manifest_sha256 {
            Ok(())
        } else {
            Err(format!(
                "{owner} render-target content authority differs from production frame"
            ))
        }
    };
    if let Some(content) = &target.persistent_content {
        check(content)?;
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                check(content)?;
            }
        }
    }
    Ok(())
}

fn collect_static_draw_used_content(draw: &StaticMapDraw) -> Vec<UsedContentKey> {
    fn push_unique(out: &mut Vec<UsedContentKey>, content: &UsedContentKey) {
        if !out.contains(content) {
            out.push(content.clone());
        }
    }
    fn collect_source(out: &mut Vec<UsedContentKey>, source: &BufferSource) {
        if let BufferSource::UsedContentRange { content, .. } = source {
            push_unique(out, content);
        }
    }
    fn collect_resource(out: &mut Vec<UsedContentKey>, resource: &TextureResourceDesc) {
        for subresource in &resource.ordered_subresources {
            collect_source(out, &subresource.source);
        }
    }

    let mut out = Vec::new();
    push_unique(&mut out, &draw.shader_program.source);
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content, resource, ..
            } => {
                push_unique(&mut out, content);
                collect_resource(&mut out, resource);
            }
            StaticShaderResourceSource::AuthenticatedSnapshot { .. } => {}
            StaticShaderResourceSource::PreparedTarget { resource, .. }
            | StaticShaderResourceSource::SnowMudTexture { resource, .. }
            | StaticShaderResourceSource::PoliticalTexture { resource, .. }
            | StaticShaderResourceSource::TiledLightIndex { resource, .. }
            | StaticShaderResourceSource::TiledLightData { resource, .. } => {
                collect_resource(&mut out, resource);
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for binding in &geometry.ordered_vertex_buffers {
            collect_source(&mut out, &binding.source);
        }
        if let Some(binding) = &geometry.index_buffer {
            collect_source(&mut out, &binding.source);
        }
    }
    for target in draw
        .output_state
        .ordered_color_targets
        .iter()
        .chain(draw.output_state.depth_target.iter())
    {
        if let Some(content) = &target.persistent_content {
            push_unique(&mut out, content);
        }
        if let Some(exact) = &target.exact_resource {
            for initial in &exact.ordered_initial_subresources {
                if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source
                {
                    push_unique(&mut out, content);
                }
            }
        }
    }
    out
}

fn validate_runtime_draw_vfs_authority(
    identity_content: &[&UsedContentKey],
    shader_program: &ShaderProgram,
    texture_bindings: &[TextureBinding],
    geometry: &GeometryBinding,
    output_state: &RenderOutputState,
    expected_manifest_sha256: &str,
    owner: &str,
) -> Result<(), String> {
    let check = |content: &UsedContentKey| {
        if content.effective_vfs_manifest_sha256 == expected_manifest_sha256 {
            Ok(())
        } else {
            Err(format!(
                "{owner} content authority differs from production frame"
            ))
        }
    };
    for content in identity_content {
        check(content)?;
    }
    check(&shader_program.source)?;
    for binding in texture_bindings {
        check(&binding.content)?;
        validate_texture_resource_vfs_authority(&binding.resource, &check)?;
    }
    for binding in &geometry.ordered_vertex_buffers {
        validate_buffer_source_vfs_authority(&binding.source, &check)?;
    }
    if let Some(binding) = &geometry.index_buffer {
        validate_buffer_source_vfs_authority(&binding.source, &check)?;
    }
    for target in output_state
        .ordered_color_targets
        .iter()
        .chain(output_state.depth_target.iter())
    {
        validate_render_target_vfs_authority(target, expected_manifest_sha256, owner)?;
    }
    Ok(())
}

fn validate_texture_resource_vfs_authority<F>(
    resource: &TextureResourceDesc,
    check: &F,
) -> Result<(), String>
where
    F: Fn(&UsedContentKey) -> Result<(), String>,
{
    for subresource in &resource.ordered_subresources {
        validate_buffer_source_vfs_authority(&subresource.source, check)?;
    }
    Ok(())
}

fn validate_buffer_source_vfs_authority<F>(source: &BufferSource, check: &F) -> Result<(), String>
where
    F: Fn(&UsedContentKey) -> Result<(), String>,
{
    if let BufferSource::UsedContentRange { content, .. } = source {
        check(content)?;
    }
    Ok(())
}

impl StaticGeometrySource {
    fn validate(&self, pipeline: &PipelineDesc) -> Result<(), String> {
        match self {
            Self::Exact { geometry } => geometry.validate(pipeline, false),
            Self::InterpreterOutput {
                interpreter_token,
                ordered_input_content_keys,
                ordered_vertex_buffers,
                index_buffer,
                stream_output_targets,
                draw,
            } => {
                if interpreter_token.trim().is_empty() || ordered_vertex_buffers.is_empty() {
                    return Err(
                        "static geometry interpreter/token or vertex output is empty".into(),
                    );
                }
                let mut inputs = BTreeSet::new();
                for key in ordered_input_content_keys {
                    if key.trim().is_empty() || !inputs.insert(key) {
                        return Err(
                            "static geometry interpreter input key is blank or duplicated".into(),
                        );
                    }
                }
                let mut slots = BTreeSet::new();
                for vertex in ordered_vertex_buffers {
                    if vertex.stride_bytes == 0
                        || vertex.vertex_count == 0
                        || !slots.insert(vertex.input_slot)
                    {
                        return Err(
                            "interpreter vertex output has an invalid/duplicate slot or count"
                                .into(),
                        );
                    }
                    check_sha256(&vertex.payload_sha256, None)?;
                }
                let layout_slots = pipeline
                    .input_layout
                    .iter()
                    .map(|element| element.input_slot)
                    .collect::<BTreeSet<_>>();
                if layout_slots != slots {
                    return Err("interpreter vertex slots do not exactly match input layout".into());
                }
                match (index_buffer, pipeline.index_format) {
                    (None, IndexFormat::None) => {}
                    (Some(index), format)
                        if format != IndexFormat::None
                            && index.format == format
                            && index.index_count != 0 =>
                    {
                        check_sha256(&index.payload_sha256, None)?;
                    }
                    _ => {
                        return Err(
                            "interpreter index output disagrees with pipeline index format".into(),
                        )
                    }
                }
                validate_stream_output_targets(stream_output_targets)?;
                validate_draw_nonzero(draw)?;
                let indexed = matches!(
                    draw,
                    DrawCommand::DrawIndexed { .. } | DrawCommand::DrawIndexedInstanced { .. }
                );
                if indexed != index_buffer.is_some() {
                    return Err("interpreter draw/index-output binding mismatch".into());
                }
                Ok(())
            }
        }
    }
}

fn validate_stream_output_targets(targets: &[StreamOutputTargetBinding]) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    let mut buffers = BTreeSet::new();
    for target in targets {
        if target.buffer_identity.trim().is_empty()
            || target.buffer_byte_width == 0
            || !slots.insert(target.slot)
            || !buffers.insert(target.buffer_identity.as_str())
            || (target.byte_offset != u32::MAX && target.byte_offset > target.buffer_byte_width)
        {
            return Err(
                "stream-output target has invalid/duplicate identity, slot, size, or offset".into(),
            );
        }
    }
    Ok(())
}

fn validate_draw_nonzero(draw: &DrawCommand) -> Result<(), String> {
    let nonzero = match draw {
        DrawCommand::Draw { vertex_count, .. } => *vertex_count != 0,
        DrawCommand::DrawIndexed { index_count, .. } => *index_count != 0,
        DrawCommand::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            ..
        } => *vertex_count_per_instance != 0 && *instance_count != 0,
        DrawCommand::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            ..
        } => *index_count_per_instance != 0 && *instance_count != 0,
    };
    if nonzero {
        Ok(())
    } else {
        Err("draw command has a zero vertex/index/instance count".into())
    }
}

fn validate_static_target(target: &RenderTargetDesc, depth: bool) -> Result<(), String> {
    target.prepared_target.validate()?;
    if target.width == 0 || target.height == 0 || target.sample_count == 0 {
        return Err("static render target is zero-sized".into());
    }
    if let Some(content) = &target.persistent_content {
        content.validate()?;
    }
    if depth && target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer {
        return Err("swapchain backbuffer cannot be a depth target".into());
    }
    match &target.exact_resource {
        Some(exact) => exact.validate_against(target, depth)?,
        None => {
            if target.mip_level != 0 || target.persistent_content.is_some() {
                return Err(
                    "persistent/nonzero-mip render target lacks exact resource/view/init descriptor"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

impl RenderTargetExactResource {
    fn validate_against(&self, selected: &RenderTargetDesc, depth: bool) -> Result<(), String> {
        self.resource.validate()?;
        if selected.mip_level >= self.resource.mip_levels
            || selected.sample_count != self.resource.sample_count
            || selected.sample_quality != self.resource.sample_quality
            || selected.width != mip_extent(self.resource.width, selected.mip_level)
            || selected.height != mip_extent(self.resource.height, selected.mip_level)
        {
            return Err(
                "selected render-target mip/extent/sample differs from exact resource".into(),
            );
        }

        let mut render_target_view = None;
        let mut depth_stencil_view = None;
        let mut shader_resource_view_count = 0usize;
        for view in &self.ordered_views {
            match view {
                RenderTargetViewDesc::RenderTarget {
                    dxgi_format_u32,
                    view,
                } => {
                    if *dxgi_format_u32 == 0
                        || self.resource.bind_flags_u32 & 0x20 == 0
                        || render_target_view.is_some()
                    {
                        return Err("exact render target repeats RTV view".into());
                    }
                    view.validate_against(&self.resource)?;
                    render_target_view = Some((*dxgi_format_u32, view.selected_mip()));
                }
                RenderTargetViewDesc::DepthStencil {
                    dxgi_format_u32,
                    flags_u32,
                    view,
                } => {
                    if *dxgi_format_u32 == 0
                        || *flags_u32 & !0x3 != 0
                        || self.resource.bind_flags_u32 & 0x40 == 0
                        || depth_stencil_view.is_some()
                    {
                        return Err("exact render target repeats DSV view".into());
                    }
                    view.validate_against(&self.resource)?;
                    depth_stencil_view = Some((*dxgi_format_u32, view.selected_mip()));
                }
                RenderTargetViewDesc::ShaderResource { view } => {
                    if self.resource.bind_flags_u32 & 0x8 == 0 {
                        return Err(
                            "exact render-target SRV lacks shader-resource bind flag".into()
                        );
                    }
                    shader_resource_view_count += 1;
                    view.validate()?;
                    view.validate_against(&self.resource.as_texture_resource_desc())?;
                }
            }
        }
        let selected_view = if depth {
            if render_target_view.is_some() || depth_stencil_view.is_none() {
                return Err("depth output exact descriptor lacks sole DSV view".into());
            }
            depth_stencil_view
        } else {
            if depth_stencil_view.is_some() || render_target_view.is_none() {
                return Err("color output exact descriptor lacks sole RTV view".into());
            }
            render_target_view
        }
        .expect("required view checked");
        if selected_view.0 == 0
            || selected_view.0 != selected.dxgi_format_u32
            || selected_view.1 != selected.mip_level
        {
            return Err(
                "selected render-target format/mip differs from exact attachment view".into(),
            );
        }
        if shader_resource_view_count > 1 {
            return Err("exact render target repeats SRV view".into());
        }

        let subresource_count = self
            .resource
            .mip_levels
            .checked_mul(self.resource.array_size)
            .ok_or("render-target subresource count overflows")?;
        let mut previous = None;
        for initial in &self.ordered_initial_subresources {
            if initial.subresource_ordinal >= subresource_count
                || previous.is_some_and(|ordinal| initial.subresource_ordinal <= ordinal)
            {
                return Err("render-target initial subresources are out of range/order".into());
            }
            previous = Some(initial.subresource_ordinal);
            initial.validate()?;
        }
        if self.resource.usage_u32 == 1
            && self.ordered_initial_subresources.len() != subresource_count as usize
        {
            return Err("immutable render target lacks complete initial subresources".into());
        }
        if let Some(persistent) = &selected.persistent_content {
            let joined = self.ordered_initial_subresources.iter().any(|initial| {
                matches!(
                    &initial.source,
                    RenderTargetInitialSource::UsedContentRange { content, .. }
                        if content == persistent
                )
            });
            if !joined {
                return Err(
                    "persistent render target is not joined to exact initial content".into(),
                );
            }
        }
        Ok(())
    }
}

impl RenderTargetResourceDesc {
    fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.depth == 0
            || self.mip_levels == 0
            || self.array_size == 0
            || self.dxgi_format_u32 == 0
            || self.sample_count == 0
            || self.usage_u32 > 3
        {
            return Err("exact render-target resource descriptor is zero/unknown".into());
        }
        match self.dimension {
            TextureResourceDimension::Texture1d
                if self.height != 1 || self.depth != 1 || self.sample_count != 1 =>
            {
                return Err("Texture1D render-target resource dimensions are invalid".into())
            }
            TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube
                if self.depth != 1 =>
            {
                return Err("Texture2D/cube render-target depth is not one".into())
            }
            TextureResourceDimension::Texture3d
                if self.array_size != 1 || self.sample_count != 1 =>
            {
                return Err("Texture3D render-target array/sample descriptor is invalid".into())
            }
            _ => {}
        }
        if self.dimension == TextureResourceDimension::TextureCube
            && (self.array_size < 6 || self.array_size % 6 != 0)
        {
            return Err("cube render-target array size is not a face multiple".into());
        }
        if self.sample_count > 1 && self.mip_levels != 1 {
            return Err("multisampled render target has more than one mip".into());
        }
        match self.usage_u32 {
            0 | 1 if self.cpu_access_flags_u32 != 0 => {
                return Err("default/immutable render target has CPU access flags".into())
            }
            2 if self.cpu_access_flags_u32 & 0x1_0000 == 0 => {
                return Err("dynamic render target lacks D3D11_CPU_ACCESS_WRITE".into())
            }
            3 if self.bind_flags_u32 != 0 => {
                return Err("staging render target has bind flags".into())
            }
            _ => {}
        }
        Ok(())
    }

    fn as_texture_resource_desc(&self) -> TextureResourceDesc {
        TextureResourceDesc {
            dimension: self.dimension,
            width: self.width,
            height: self.height,
            depth: self.depth,
            mip_levels: self.mip_levels,
            array_size: self.array_size,
            dxgi_format_u32: self.dxgi_format_u32,
            sample_count: self.sample_count,
            sample_quality: self.sample_quality,
            usage_u32: self.usage_u32,
            bind_flags_u32: self.bind_flags_u32,
            cpu_access_flags_u32: self.cpu_access_flags_u32,
            misc_flags_u32: self.misc_flags_u32,
            ordered_subresources: vec![],
        }
    }
}

impl RenderTargetAttachmentView {
    fn selected_mip(&self) -> u32 {
        match self {
            Self::Texture1d { mip_slice }
            | Self::Texture1dArray { mip_slice, .. }
            | Self::Texture2d { mip_slice }
            | Self::Texture2dArray { mip_slice, .. }
            | Self::Texture3d { mip_slice, .. } => *mip_slice,
            Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. } => 0,
        }
    }

    fn validate_against(&self, resource: &RenderTargetResourceDesc) -> Result<(), String> {
        validate_attachment_view(
            self.selected_mip(),
            attachment_array_range(self),
            matches!(
                self,
                Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. }
            ),
            resource,
            matches!(self, Self::Texture1d { .. } | Self::Texture1dArray { .. }),
            matches!(self, Self::Texture3d { .. }),
        )?;
        if let Self::Texture3d {
            first_w_slice,
            w_size,
            mip_slice,
        } = self
        {
            let depth = mip_extent(resource.depth, *mip_slice);
            if *w_size == 0
                || first_w_slice
                    .checked_add(*w_size)
                    .is_none_or(|end| end > depth)
            {
                return Err("Texture3D RTV W-slice range is invalid".into());
            }
        }
        Ok(())
    }
}

impl DepthStencilAttachmentView {
    fn selected_mip(&self) -> u32 {
        match self {
            Self::Texture1d { mip_slice }
            | Self::Texture1dArray { mip_slice, .. }
            | Self::Texture2d { mip_slice }
            | Self::Texture2dArray { mip_slice, .. } => *mip_slice,
            Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. } => 0,
        }
    }

    fn validate_against(&self, resource: &RenderTargetResourceDesc) -> Result<(), String> {
        validate_attachment_view(
            self.selected_mip(),
            depth_attachment_array_range(self),
            matches!(
                self,
                Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. }
            ),
            resource,
            matches!(self, Self::Texture1d { .. } | Self::Texture1dArray { .. }),
            false,
        )
    }
}

fn validate_attachment_view(
    mip: u32,
    array_range: Option<(u32, u32)>,
    multisampled: bool,
    resource: &RenderTargetResourceDesc,
    one_dimensional: bool,
    three_dimensional: bool,
) -> Result<(), String> {
    if mip >= resource.mip_levels || multisampled != (resource.sample_count > 1) {
        return Err("render-target attachment view mip/sample is invalid".into());
    }
    if one_dimensional != (resource.dimension == TextureResourceDimension::Texture1d)
        || three_dimensional != (resource.dimension == TextureResourceDimension::Texture3d)
        || (!one_dimensional
            && !three_dimensional
            && !matches!(
                resource.dimension,
                TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube
            ))
    {
        return Err("render-target attachment view dimension differs from resource".into());
    }
    if let Some((first, count)) = array_range {
        if count == 0
            || first
                .checked_add(count)
                .is_none_or(|end| end > resource.array_size)
        {
            return Err("render-target attachment array range is invalid".into());
        }
    } else if resource.array_size != 1 && !three_dimensional {
        return Err("array render target uses a non-array attachment view".into());
    }
    Ok(())
}

fn attachment_array_range(view: &RenderTargetAttachmentView) -> Option<(u32, u32)> {
    match view {
        RenderTargetAttachmentView::Texture1dArray {
            first_array_slice,
            array_size,
            ..
        }
        | RenderTargetAttachmentView::Texture2dArray {
            first_array_slice,
            array_size,
            ..
        }
        | RenderTargetAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => Some((*first_array_slice, *array_size)),
        _ => None,
    }
}

fn depth_attachment_array_range(view: &DepthStencilAttachmentView) -> Option<(u32, u32)> {
    match view {
        DepthStencilAttachmentView::Texture1dArray {
            first_array_slice,
            array_size,
            ..
        }
        | DepthStencilAttachmentView::Texture2dArray {
            first_array_slice,
            array_size,
            ..
        }
        | DepthStencilAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => Some((*first_array_slice, *array_size)),
        _ => None,
    }
}

impl RenderTargetInitialSubresource {
    fn validate(&self) -> Result<(), String> {
        if self.row_pitch == 0 || self.slice_pitch < self.row_pitch {
            return Err("render-target initial subresource pitch is invalid".into());
        }
        match &self.source {
            RenderTargetInitialSource::Inline { bytes, sha256 } => {
                if bytes.len() < self.slice_pitch as usize {
                    return Err(
                        "render-target inline initialization is shorter than slice pitch".into(),
                    );
                }
                check_sha256(sha256, Some(bytes))
            }
            RenderTargetInitialSource::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                content.validate()?;
                check_sha256(range_sha256, None)?;
                if *byte_count < self.slice_pitch as u64
                    || source_byte_offset
                        .checked_add(*byte_count)
                        .is_none_or(|end| end > content.payload_bytes)
                {
                    return Err("render-target initial content range is invalid".into());
                }
                Ok(())
            }
        }
    }
}

fn mip_extent(base: u32, mip: u32) -> u32 {
    base.checked_shr(mip).unwrap_or(0).max(1)
}

fn validate_static_output_state(output: &RenderOutputState) -> Result<(), String> {
    let viewport = &output.viewport;
    let values = [
        viewport.top_left_x_f32_bits,
        viewport.top_left_y_f32_bits,
        viewport.width_f32_bits,
        viewport.height_f32_bits,
        viewport.min_depth_f32_bits,
        viewport.max_depth_f32_bits,
    ]
    .map(f32::from_bits);
    if values.iter().any(|value| !value.is_finite())
        || values[2] <= 0.0
        || values[3] <= 0.0
        || values[4] < 0.0
        || values[5] > 1.0
        || values[4] > values[5]
    {
        return Err("static viewport is non-finite, empty, or has an invalid depth range".into());
    }
    Ok(())
}

fn validate_prepared_texture_resource(resource: &TextureResourceDesc) -> Result<(), String> {
    if resource.dimension != TextureResourceDimension::Texture2d
        || resource.width == 0
        || resource.height == 0
        || resource.depth != 1
        || resource.mip_levels == 0
        || resource.array_size != 1
        || resource.sample_count == 0
        || !resource.ordered_subresources.is_empty()
    {
        return Err("prepared render-graph texture descriptor is invalid or carries invented initialization bytes".into());
    }
    Ok(())
}

fn validate_submit_payload_texture_resource(resource: &TextureResourceDesc) -> Result<(), String> {
    validate_prepared_texture_resource(resource)?;
    if resource.sample_count != 1 {
        return Err("submit-carried dynamic texture is unexpectedly multisampled".into());
    }
    Ok(())
}

fn validate_authenticated_snapshot_texture_resource(
    resource: &TextureResourceDesc,
) -> Result<(), String> {
    resource.validate()?;
    if resource.sample_count != 1
        || resource.ordered_subresources.is_empty()
        || resource
            .ordered_subresources
            .iter()
            .any(|subresource| !matches!(&subresource.source, BufferSource::Inline { .. }))
    {
        return Err(
            "authenticated draw-time texture snapshot must carry complete inline non-MSAA subresources"
                .into(),
        );
    }
    Ok(())
}

impl ShaderProgram {
    fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        if self.producer_registration_root_rva == Some(0)
            || self.effect_token.trim().is_empty()
            || self.vertex_entry_token.trim().is_empty()
            || self.pixel_entry_token.trim().is_empty()
            || self
                .ordered_defines
                .iter()
                .any(|define| define.name.trim().is_empty())
        {
            return Err("shader program has a blank token/define".into());
        }
        self.compiled_vertex_shader.validate("vertex")?;
        self.compiled_pixel_shader.validate("pixel")?;
        for (name, stage) in [
            ("geometry", self.geometry_shader.as_ref()),
            ("hull", self.hull_shader.as_ref()),
            ("domain", self.domain_shader.as_ref()),
            ("compute", self.compute_shader.as_ref()),
        ] {
            if let Some(stage) = stage {
                if stage.entry_token.trim().is_empty() {
                    return Err(format!("compiled {name} shader has a blank entry token"));
                }
                stage.compiled.validate(name)?;
            }
        }
        Ok(())
    }
}

impl CompiledShaderPayload {
    fn validate(&self, stage: &str) -> Result<(), String> {
        if self.compiler_identity.trim().is_empty()
            || self.target_profile.trim().is_empty()
            || self.bytecode.is_empty()
        {
            return Err(format!(
                "compiled {stage} shader has blank provenance or bytecode"
            ));
        }
        check_sha256(&self.sha256, Some(&self.bytecode))
    }
}

impl EntityContentIdentity {
    fn validate(&self) -> Result<(), String> {
        self.resolved_entity_asset.validate()?;
        self.mesh.validate()?;
        self.native_selection.validate()?;
        check_sha256(&self.native_selection_sha256, None)?;
        if self.native_selection.computed_sha256()? != self.native_selection_sha256 {
            return Err("entity selector-native identity SHA-256 mismatch".into());
        }
        Ok(())
    }
}

impl ParticleContentIdentity {
    pub fn computed_sha256(&self) -> Result<String, String> {
        canonical_json_sha256(self)
    }

    fn validate(&self) -> Result<(), String> {
        self.definition_content.validate()?;
        self.texture_content.validate()?;
        if self.authored_shader_token.trim().is_empty()
            || self.engine_effect_token.trim().is_empty()
        {
            return Err("particle content identity has a blank shader/effect token".into());
        }
        if self.atlas_columns_u32 == 0 || self.atlas_rows_u32 == 0 {
            return Err("particle content identity has zero authored atlas dimensions".into());
        }
        Ok(())
    }
}

impl EntityNativeSelection {
    pub fn computed_sha256(&self) -> Result<String, String> {
        canonical_json_sha256(self)
    }

    pub fn packed_state_flags(&self) -> u32 {
        u32::from(self.entity_variant_u8)
            | (u32::from(self.mesh_draw_variant_u8) << 8)
            | (u32::from(self.bind_renderer_target_u8) << 16)
            | (u32::from(self.alternate_list_u8) << 24)
    }

    fn validate(&self) -> Result<(), String> {
        if self.entity_registration_token.trim().is_empty() {
            return Err("entity selector-native identity has a blank registration token".into());
        }
        Ok(())
    }
}

impl PipelineDesc {
    fn validate(&self) -> Result<(), String> {
        if self.blend.write_mask_rgba_u8 & !0x0f != 0
            || self.blend.additional_targets.len() > 7
            || (!self.blend.independent_blend_enabled && !self.blend.additional_targets.is_empty())
            || self
                .blend
                .additional_targets
                .iter()
                .any(|target| target.write_mask_rgba_u8 & !0x0f != 0)
            || self
                .blend
                .blend_factor_rgba_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("pipeline blend write mask has unknown bits".into());
        }
        for bits in [
            self.rasterizer.depth_bias_clamp_f32_bits,
            self.rasterizer.slope_scaled_depth_bias_f32_bits,
        ] {
            if !f32::from_bits(bits).is_finite() {
                return Err("pipeline rasterizer bias is not finite".into());
            }
        }
        let mut sampler_slots = BTreeSet::new();
        for sampler in &self.samplers {
            if sampler.semantic_token.trim().is_empty()
                || !sampler_slots.insert((sampler.stage, sampler.slot))
                || !(1..=16).contains(&sampler.max_anisotropy)
                || !f32::from_bits(sampler.mip_lod_bias_f32_bits).is_finite()
                || !f32::from_bits(sampler.min_lod_f32_bits).is_finite()
                || !f32::from_bits(sampler.max_lod_f32_bits).is_finite()
                || f32::from_bits(sampler.min_lod_f32_bits)
                    > f32::from_bits(sampler.max_lod_f32_bits)
                || sampler
                    .border_color_rgba_f32_bits
                    .iter()
                    .any(|bits| !f32::from_bits(*bits).is_finite())
            {
                return Err("pipeline sampler slot is duplicate or unnamed".into());
            }
        }
        let mut elements = BTreeSet::new();
        for element in &self.input_layout {
            if element.semantic_token.trim().is_empty()
                || !elements.insert((
                    element.input_slot,
                    element.semantic_token.as_str(),
                    element.semantic_index,
                ))
            {
                return Err("pipeline input element is duplicate or unnamed".into());
            }
            if element.per_instance != (element.instance_step_rate != 0) {
                return Err("pipeline input element instance step-rate mismatch".into());
            }
        }
        match self.index_format {
            IndexFormat::R16Uint if self.static_indices_u16.is_empty() => {
                Err("R16 pipeline has no static indices".into())
            }
            IndexFormat::R32Uint if !self.static_indices_u16.is_empty() => {
                Err("R32 pipeline cannot carry u16 static indices".into())
            }
            IndexFormat::None if !self.static_indices_u16.is_empty() => {
                Err("non-indexed pipeline carries static indices".into())
            }
            _ => Ok(()),
        }
    }
}

impl RenderOutputState {
    fn validate(&self) -> Result<(), String> {
        if self.ordered_color_targets.is_empty() {
            return Err("render output state has no color target".into());
        }
        let mut bindings = BTreeSet::new();
        for target in &self.ordered_color_targets {
            validate_static_target(target, false)?;
            if !bindings.insert(&target.prepared_target) {
                return Err("render target descriptor is duplicated".into());
            }
        }
        if let Some(target) = &self.depth_target {
            validate_static_target(target, true)?;
            if !bindings.insert(&target.prepared_target) {
                return Err("render target descriptor is duplicated".into());
            }
        }
        if let Some(scissor) = &self.scissor {
            if scissor.right_i32 <= scissor.left_i32 || scissor.bottom_i32 <= scissor.top_i32 {
                return Err("render scissor is empty or inverted".into());
            }
        }
        Ok(())
    }
}

fn validate_texture_bindings(bindings: &[TextureBinding]) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    for binding in bindings {
        if binding.semantic_token.trim().is_empty() || !slots.insert((binding.stage, binding.slot))
        {
            return Err("texture binding slot is duplicate or unnamed".into());
        }
        binding.content.validate()?;
        binding.resource.validate()?;
        binding.view.validate()?;
        binding.view.validate_against(&binding.resource)?;
    }
    Ok(())
}

impl TextureResourceDesc {
    fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.depth == 0
            || self.mip_levels == 0
            || self.array_size == 0
            || self.sample_count != 1
        {
            return Err("texture resource has invalid dimensions/mips/array/samples".into());
        }
        match self.dimension {
            TextureResourceDimension::Texture1d if self.height != 1 || self.depth != 1 => {
                return Err("Texture1D resource has non-unit height/depth".into())
            }
            TextureResourceDimension::Texture2d if self.depth != 1 => {
                return Err("Texture2D resource has non-unit depth".into())
            }
            TextureResourceDimension::Texture3d if self.array_size != 1 => {
                return Err("Texture3D resource has an array size other than one".into())
            }
            TextureResourceDimension::TextureCube
                if self.depth != 1 || self.array_size % 6 != 0 =>
            {
                return Err("texture cube does not contain complete six-face arrays".into())
            }
            _ => {}
        }
        let expected = match self.dimension {
            TextureResourceDimension::Texture3d => self.mip_levels,
            _ => self
                .mip_levels
                .checked_mul(self.array_size)
                .ok_or_else(|| "texture subresource count overflows".to_string())?,
        };
        if usize::try_from(expected).ok() != Some(self.ordered_subresources.len()) {
            return Err("texture resource does not carry every ordered subresource".into());
        }
        for (ordinal, subresource) in self.ordered_subresources.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| "texture subresource ordinal overflows u32".to_string())?;
            let expected_mip = ordinal % self.mip_levels;
            let expected_array = if self.dimension == TextureResourceDimension::Texture3d {
                0
            } else {
                ordinal / self.mip_levels
            };
            if subresource.subresource_index != ordinal
                || subresource.mip_level != expected_mip
                || subresource.array_slice != expected_array
                || subresource.row_pitch == 0
                || subresource.slice_pitch == 0
            {
                return Err("texture subresource order/index/pitch mismatch".into());
            }
            subresource.source.validate(false)?;
        }
        Ok(())
    }
}

impl ShaderResourceViewDesc {
    fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Buffer { element_count, .. } => *element_count != 0,
            Self::Texture1d { mip_levels, .. }
            | Self::Texture2d { mip_levels, .. }
            | Self::Texture3d { mip_levels, .. }
            | Self::TextureCube { mip_levels, .. } => *mip_levels != 0,
            Self::Texture1dArray {
                mip_levels,
                array_size,
                ..
            }
            | Self::Texture2dArray {
                mip_levels,
                array_size,
                ..
            } => *mip_levels != 0 && *array_size != 0,
            Self::Texture2dMultisampled { .. } => true,
            Self::Texture2dMultisampledArray { array_size, .. } => *array_size != 0,
            Self::TextureCubeArray {
                mip_levels,
                cube_count,
                ..
            } => *mip_levels != 0 && *cube_count != 0,
        };
        if valid {
            Ok(())
        } else {
            Err("shader-resource view exposes an empty element/mip/array range".into())
        }
    }

    fn validate_against(&self, resource: &TextureResourceDesc) -> Result<(), String> {
        let range_ok = |first: u32, count: u32, total: u32| {
            count != 0 && first.checked_add(count).is_some_and(|end| end <= total)
        };
        let compatible = match self {
            Self::Buffer { .. } => false,
            Self::Texture1d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture1d
                    && resource.array_size == 1
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::Texture1dArray {
                most_detailed_mip,
                mip_levels,
                first_array_slice,
                array_size,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture1d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && range_ok(*first_array_slice, *array_size, resource.array_size)
            }
            Self::Texture2d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture2d
                    && resource.array_size == 1
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::Texture2dArray {
                most_detailed_mip,
                mip_levels,
                first_array_slice,
                array_size,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture2d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && range_ok(*first_array_slice, *array_size, resource.array_size)
            }
            Self::Texture2dMultisampled { .. } | Self::Texture2dMultisampledArray { .. } => false,
            Self::Texture3d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture3d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::TextureCube {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::TextureCube
                    && resource.array_size == 6
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::TextureCubeArray {
                most_detailed_mip,
                mip_levels,
                first_face,
                cube_count,
                ..
            } => {
                resource.dimension == TextureResourceDimension::TextureCube
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && first_face % 6 == 0
                    && cube_count
                        .checked_mul(6)
                        .is_some_and(|faces| range_ok(*first_face, faces, resource.array_size))
            }
        };
        if compatible {
            Ok(())
        } else {
            Err("shader-resource view is incompatible with its exact texture resource".into())
        }
    }
}

impl PreparedRenderTarget {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl GeometryBinding {
    fn validate(
        &self,
        pipeline: &PipelineDesc,
        allow_particle_records: bool,
    ) -> Result<(), String> {
        if self.ordered_vertex_buffers.is_empty() {
            return Err("geometry has no vertex buffers".into());
        }
        let mut buffer_slots = BTreeSet::new();
        for buffer in &self.ordered_vertex_buffers {
            if buffer.stride_bytes == 0 || !buffer_slots.insert(buffer.input_slot) {
                return Err("geometry vertex-buffer slot is duplicate or has zero stride".into());
            }
            buffer.source.validate(allow_particle_records)?;
            let slot_is_instance = pipeline
                .input_layout
                .iter()
                .filter(|element| element.input_slot == buffer.input_slot)
                .all(|element| element.per_instance);
            if matches!(buffer.source, BufferSource::ParticleInstanceRecords) && !slot_is_instance {
                return Err(
                    "particle instance records are bound to a per-vertex input slot".into(),
                );
            }
        }
        let layout_slots: BTreeSet<_> = pipeline
            .input_layout
            .iter()
            .map(|element| element.input_slot)
            .collect();
        if layout_slots != buffer_slots {
            return Err(
                "geometry vertex-buffer slots do not exactly match the input layout".into(),
            );
        }
        match (&self.index_buffer, pipeline.index_format) {
            (None, IndexFormat::None) => {}
            (Some(index), format) if format != IndexFormat::None && index.format == format => {
                index.source.validate(false)?;
            }
            _ => return Err("geometry index binding disagrees with pipeline index format".into()),
        }
        validate_stream_output_targets(&self.stream_output_targets)?;
        let indexed_draw = matches!(
            self.draw,
            DrawCommand::DrawIndexed { .. } | DrawCommand::DrawIndexedInstanced { .. }
        );
        if indexed_draw != self.index_buffer.is_some() {
            return Err("draw command/index-buffer binding mismatch".into());
        }
        validate_draw_nonzero(&self.draw)?;
        Ok(())
    }
}

impl BufferSource {
    fn validate(&self, allow_particle_records: bool) -> Result<(), String> {
        match self {
            Self::Inline { bytes, sha256 } => {
                if bytes.is_empty() {
                    return Err("inline GPU buffer source is empty".into());
                }
                check_sha256(sha256, Some(bytes))
            }
            Self::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                content.validate()?;
                check_sha256(range_sha256, None)?;
                let end = source_byte_offset
                    .checked_add(*byte_count)
                    .ok_or_else(|| "content buffer range overflows".to_string())?;
                if *byte_count == 0 || end > content.payload_bytes {
                    return Err("content buffer range is empty or outside its payload".into());
                }
                Ok(())
            }
            Self::ParticleInstanceRecords if allow_particle_records => Ok(()),
            Self::ParticleInstanceRecords => {
                Err("entity geometry references particle packet instance bytes".into())
            }
        }
    }
}

fn validate_constant_buffers(bindings: &[ConstantBufferBinding]) -> Result<(), String> {
    let mut stage_slots = BTreeSet::new();
    for binding in bindings {
        if binding.stage_mask_u8 == 0
            || binding.stage_mask_u8 & !0x3f != 0
            || binding.semantic_layout.trim().is_empty()
        {
            return Err(
                "constant-buffer binding is duplicate/unnamed/has invalid stage mask".into(),
            );
        }
        for stage in 0..6 {
            if binding.stage_mask_u8 & (1 << stage) != 0
                && !stage_slots.insert((stage, binding.slot))
            {
                return Err(
                    "constant-buffer binding duplicates an already-bound stage/slot".into(),
                );
            }
        }
        let end = usize::try_from(binding.byte_offset)
            .ok()
            .and_then(|offset| {
                usize::try_from(binding.byte_count)
                    .ok()
                    .and_then(|count| offset.checked_add(count))
            })
            .ok_or_else(|| "constant-buffer byte range overflow".to_string())?;
        if binding.byte_count == 0 || end > binding.raw_bytes.len() {
            return Err("constant-buffer byte range exceeds captured bytes".into());
        }
        check_sha256(&binding.sha256, Some(&binding.raw_bytes))?;
    }
    Ok(())
}

fn validate_output_refs(
    refs: &[RenderOutputRef],
    particle_outputs: &BTreeSet<RenderOutputRef>,
    entity_outputs: &BTreeSet<RenderOutputRef>,
    owner: &str,
) -> Result<(), String> {
    let mut unique = BTreeSet::new();
    for output in refs {
        if !unique.insert(output.clone()) {
            return Err(format!("{owner} repeats a submitted output join"));
        }
        let exists = match output {
            RenderOutputRef::Particle { .. } => particle_outputs.contains(output),
            RenderOutputRef::Entity { .. } => entity_outputs.contains(output),
        };
        if !exists {
            return Err(format!("{owner} references an absent submitted output"));
        }
    }
    Ok(())
}

fn check_sha256(text: &str, bytes: Option<&[u8]>) -> Result<(), String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("SHA-256 is not 64 lowercase hexadecimal characters".into());
    }
    if let Some(bytes) = bytes {
        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual != text {
            return Err("SHA-256 payload mismatch".into());
        }
    }
    Ok(())
}

fn check_decimal_u64(text: &str, field: &str) -> Result<(), String> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) || text.parse::<u64>().is_err()
    {
        return Err(format!("{field} is not canonical decimal u64"));
    }
    Ok(())
}

fn check_sequence(value: u64, first: u64, last: u64, field: &str) -> Result<(), String> {
    if value < first || value > last {
        return Err(format!("{field} sequence is outside frame range"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    /// The admitted effective-VFS manifest root every fixture in this module is
    /// sealed against. It is not a free constant: `content()` below stamps this
    /// exact digest into every `UsedContentKey` it mints, and
    /// `ProductionRenderFrame::validate` refuses any content join whose root
    /// differs from the admitted one it is handed. Changing either without the
    /// other turns every fixture in the module into a content-authority refusal.
    const TEST_EFFECTIVE_VFS_MANIFEST_SHA256: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn weather_big_positions_fall_back_only_when_big_vector_is_empty() {
        assert_eq!(
            weather_selected_variant_class(WeatherPositionClass::Big, true),
            WeatherPositionClass::Big
        );
        assert_eq!(
            weather_selected_variant_class(WeatherPositionClass::Big, false),
            WeatherPositionClass::Small
        );
        assert_eq!(
            weather_selected_variant_class(WeatherPositionClass::Small, true),
            WeatherPositionClass::Small
        );
        assert_eq!(
            weather_selected_variant_class(WeatherPositionClass::Small, false),
            WeatherPositionClass::Small
        );
    }

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn content(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "2".repeat(64),
            stored_game_relative_path: path.into(),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "c".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 1,
        }
    }

    fn entity_content_identity() -> EntityContentIdentity {
        let native_selection = EntityNativeSelection {
            entity_registration_token: "unit".into(),
            entity_group_index: 1,
            mesh_draw_list_index: 2,
            entity_variant_u8: 3,
            mesh_draw_variant_u8: 4,
            pass_selector_i32: -5,
            alternate_list_u8: 6,
            bind_renderer_target_u8: 7,
        };
        let native_selection_sha256 = native_selection.computed_sha256().unwrap();
        EntityContentIdentity {
            resolved_entity_asset: content("gfx/entities/unit.asset"),
            mesh: content("gfx/models/unit.mesh"),
            native_selection,
            native_selection_sha256,
        }
    }

    fn particle_content_identity(selector: u32) -> ParticleContentIdentity {
        ParticleContentIdentity {
            definition_content: content("gfx/particles/smoke.asset"),
            subsystem_selector_u32: selector,
            texture_content: content("gfx/textures/particle.dds"),
            authored_shader_token: "ParticleAdditive".into(),
            engine_effect_token: "ParticleAdditive".into(),
            atlas_columns_u32: 1,
            atlas_rows_u32: 1,
            atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
            atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
        }
    }

    fn exact_hud_pipeline_draw() -> StaticMapDraw {
        let shader = shader();
        let texture = texture();
        let mut output = output_state();
        output.ordered_color_targets[0].prepared_target = PreparedRenderTarget::SwapchainBackBuffer;
        let mut draw = StaticMapDraw {
            draw_token: "hud.exact_pipeline_probe".into(),
            shader_program: shader.clone(),
            ordered_shader_resources: vec![StaticShaderResourceBinding {
                stage: texture.stage,
                slot: texture.slot,
                semantic_token: texture.semantic_token,
                source: StaticShaderResourceSource::UsedContent {
                    content: texture.content.clone(),
                    resource: texture.resource,
                    view: texture.view,
                },
            }],
            pipeline: pipeline(),
            geometry: StaticGeometrySource::Exact {
                geometry: entity_geometry(),
            },
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output,
            payload_sha256: String::new(),
        };
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        draw
    }

    fn production_frame_with_selected_present() -> ProductionRenderFrame {
        ProductionRenderFrame {
            schema: SCHEMA.into(),
            update_index: 16_959_360,
            first_sequence: 40,
            last_sequence: 42,
            presentation_qpc_frequency_hz: 10_000_000,
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                11,
                0,
                60_759_359,
                500,
                600,
            ),
            snow_mud_texture: None,
            submits: vec![RenderSubmit {
                global_order: 1,
                camera_carrier: None,
                ordered_static_map_programs: vec![],
                authenticated_texture_snapshots: vec![],
                s1_shadow: None,
                particles: vec![],
                particle_packet_set_sha256: "5".repeat(64),
                entities: vec![],
                rail_construction_arrows: vec![],
                ordered_render_operations: vec![],
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m03_arrows: None,
            rail_supply_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            authenticated_ui_pass: None,
        }
    }

    fn census_membership_row(
        sequence: u64,
        update_index: u64,
        strategic_region_id: u32,
        snow_active: bool,
    ) -> WeatherMembership {
        WeatherMembership {
            sequence,
            update_index,
            strategic_region_id,
            rain_light_active: false,
            rain_heavy_active: false,
            snow_active,
            blizzard_active: false,
            sandstorm_active: false,
            arctic_water_active: false,
            region_value_i64_fixed_1e5: 123_456,
            transition_expiry_game_hour_i32: 60_759_400,
            airborne_runtime_code: if snow_active { 4 } else { 0 },
            selection_seed_words: None,
            visual_epoch: None,
            weather_effect_token: None,
            weather_position_source_content: content("terrain/weather/baseline.txt"),
            ordered_variant_content: vec![],
            ordered_positions: vec![],
            ordered_weather_instances: vec![],
            visibility_updates: vec![],
            membership_sha256: "d".repeat(64),
        }
    }

    /// kills: carrier.production_frame.checkpoint -- the derived per-region
    /// census equals exactly what the retired RenderCheckpoint
    /// weather_state_and_ordered_membership field carried for these sealed
    /// submit rows (latest row per strategic region, region order).
    #[test]
    fn derived_weather_census_equals_the_retired_checkpoint_carrier() {
        let submits = vec![
            RenderSubmit {
                global_order: 1,
                camera_carrier: None,
                ordered_static_map_programs: vec![],
                authenticated_texture_snapshots: vec![],
                s1_shadow: None,
                particles: vec![],
                particle_packet_set_sha256: "5".repeat(64),
                entities: vec![],
                rail_construction_arrows: vec![],
                ordered_render_operations: vec![],
                weather_membership: vec![
                    census_membership_row(41, 16_959_360, 7, false),
                    census_membership_row(42, 16_959_360, 3, true),
                ],
                political_textures: vec![],
            },
            RenderSubmit {
                global_order: 2,
                camera_carrier: None,
                ordered_static_map_programs: vec![],
                authenticated_texture_snapshots: vec![],
                s1_shadow: None,
                particles: vec![],
                particle_packet_set_sha256: "6".repeat(64),
                entities: vec![],
                rail_construction_arrows: vec![],
                ordered_render_operations: vec![],
                weather_membership: vec![
                    census_membership_row(43, 16_959_360, 7, true),
                    census_membership_row(44, 16_959_360, 5, false),
                ],
                political_textures: vec![],
            },
        ];
        // The retired carrier said exactly this: latest row per region
        // (region 3 seq 42, region 5 seq 44, region 7 seq 43), in region
        // order -- not the first rows, not submission order.
        let retired_checkpoint_rows = vec![
            census_membership_row(42, 16_959_360, 3, true),
            census_membership_row(44, 16_959_360, 5, false),
            census_membership_row(43, 16_959_360, 7, true),
        ];
        assert_eq!(
            derive_weather_checkpoint_census(&submits).unwrap(),
            retired_checkpoint_rows
        );
        let mut non_monotonic = submits.clone();
        non_monotonic[1].weather_membership[0].sequence = 41;
        assert!(derive_weather_checkpoint_census(&non_monotonic)
            .unwrap_err()
            .contains("non-monotonic"));
    }

    /// kills: carrier.production_frame.checkpoint,
    /// carrier.production_frame.display_submit_index,
    /// carrier.production_frame.authority, carrier.production_frame.m02_combat
    /// -- the wire type no longer serialises
    /// any of the four, and a frame that still carries one refuses to decode
    /// instead of having it silently dropped.
    ///
    /// The envelope half looks INSIDE `"frame"`: encode_json wraps the frame in a
    /// ProductionRenderWire ({schema, wire_schema, inline_blobs, frame}), so a
    /// retired member would ride there and a check at the envelope root would pass
    /// no matter what the type declares.
    ///
    /// The reader half runs against the undeduplicated `to_value` projection, not
    /// the encoded envelope: encode_json replaces inline byte payloads with blob
    /// references, so that form would fail to decode for a reason of its own and
    /// the refusal would prove nothing. The positive control below is what makes
    /// the refusals load-bearing.
    #[test]
    fn retired_production_frame_members_are_absent_from_the_wire() {
        let frame = production_frame_with_selected_present();
        let encoded = frame
            .encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        let wire_frame = envelope
            .get("frame")
            .and_then(serde_json::Value::as_object)
            .expect("the wire envelope carries the frame object");

        let direct = serde_json::to_value(&frame).unwrap();
        let direct_object = direct
            .as_object()
            .expect("a production frame serialises as a JSON object")
            .clone();
        assert!(
            serde_json::from_value::<ProductionRenderFrame>(direct).is_ok(),
            "the projection must decode on its own, or the refusals below prove nothing"
        );

        // m02_combat is lane RJ's XMAP14 retirement. It sits here for the same
        // reason as the other three, but it is the THIRD assertion below - the
        // injected-key decode refusal - that catches it, not either absence
        // check: the field was an Option with skip_serializing_if, so
        // re-declaring it would leave a None frame serialising byte-for-byte as
        // it does now and both absence assertions would still pass. What breaks
        // is that `"m02_combat": null` would then decode as None instead of
        // being refused by deny_unknown_fields.
        for retired in ["checkpoint", "display_submit_index", "authority", "m02_combat"] {
            assert!(
                !wire_frame.contains_key(retired),
                "retired production-frame member {retired} is back on the wire"
            );
            assert!(
                !direct_object.contains_key(retired),
                "retired production-frame member {retired} is back on the frame type"
            );
            let mut readmitted = direct_object.clone();
            readmitted.insert(retired.to_string(), serde_json::Value::Null);
            assert!(
                serde_json::from_value::<ProductionRenderFrame>(serde_json::Value::Object(
                    readmitted
                ))
                .is_err(),
                "a frame still carrying {retired} decoded instead of refusing"
            );
        }
    }

    #[test]
    fn submit_identity_and_presentation_ticks_are_derived_from_selected_present_receipt() {
        // kills: carrier.render_submit.submit_index - the sealed fixture previously carried
        // submit_index=11 on the RenderSubmit; the derived identity equals that carried value
        // because the former wire law enforced selected_candidate_id == submit.submit_index.
        // kills: carrier.render_submit.presentation_qpc_ticks - the sealed fixture previously
        // carried presentation_qpc_ticks=500 on the RenderSubmit; the derived ticks equal that
        // carried value because the former wire law enforced
        // selected_qpc_ticks == submit.presentation_qpc_ticks.
        // This test owns the DERIVATION-EQUALITY half of both claims (the derived value equals
        // what the retired field carried). The re-admission half - putting either field back on
        // the wire - is owned by `retired_submit_carriers_cannot_ride_the_wire` below, because
        // these helpers keep answering whether or not the carrier returns.
        //
        // Lane RJ merge note: the fixture this test was written against was
        // `production_frame_with_m02()`, which went with the retired M02
        // carrier. `production_frame_with_selected_present()` is the same frame
        // without that carrier - same sealed receipt (candidate 11, 500 ticks) -
        // so every assertion below still means what it was written to mean.
        let frame = production_frame_with_selected_present();
        assert_eq!(frame.selected_present.selected_candidate_id, 11);
        assert_eq!(frame.selected_present.selected_qpc_ticks, 500);
        assert_eq!(frame.retained_submit_identity(), Some(11));
        assert_eq!(frame.selected_presentation_qpc_ticks(), Some(500));
        frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();

        // The derived identity is not a free-standing string any more: submit(id) answers only
        // for the receipt's own candidate id, so the retained submit is reachable under 11 and
        // under nothing else. A foreign id has no submit to name.
        assert!(frame.submit(11).is_some());
        assert_eq!(frame.submit(12), None);
    }

    /// kills: carrier.render_submit.submit_index, carrier.render_submit.presentation_qpc_ticks,
    /// carrier.render_submit.ordered_scene_constant_buffers, carrier.render_submit.tree_runtime,
    /// carrier.render_submit.entity_presentations, carrier.render_submit.ordered_draws,
    /// carrier.render_submit.direct_visuals, carrier.render_submit.tiled_lights,
    /// carrier.render_submit.camera_carrier_ordinal -- putting any
    /// of these fields back on `RenderSubmit` puts it back on the wire, which is exactly the
    /// re-admission the schema ladder exists to prevent. Declaring the field again makes the
    /// serialized submit carry its key, and the derivation helpers next door would go on
    /// answering, so this test - not those helpers - is what fails. The second half pins the
    /// reader end: `RenderSubmit` is `deny_unknown_fields`, so a frame minted by an older or
    /// patched writer that still carries one of these keys must refuse to decode rather than
    /// drop it silently.
    #[test]
    fn retired_submit_carriers_cannot_ride_the_wire() {
        let frame = production_frame_with_selected_present();
        let value = serde_json::to_value(&frame.submits[0]).unwrap();
        let object = value
            .as_object()
            .expect("a submit serializes as a JSON object")
            .clone();
        assert!(
            serde_json::from_value::<RenderSubmit>(value).is_ok(),
            "the projection must decode on its own, or the refusals below prove nothing"
        );
        for retired in [
            "submit_index",
            "presentation_qpc_ticks",
            "ordered_scene_constant_buffers",
            "tree_runtime",
            "entity_presentations",
            "ordered_draws",
            "direct_visuals",
            "tiled_lights",
            // Lane RJ's XMAP03 retirement, added by the same law that put the
            // eight above here: the carrier ordinal is derived from the sealed
            // AuthenticatedCameraCarrier now and must not return to the wire.
            // For this one the SERIALIZED-KEY assertion is the discriminating
            // half: re-declaring it as u8 makes the key appear. Its injected-null
            // arm is not independently load-bearing, because a u8 field rejects
            // null whether or not it is declared.
            "camera_carrier_ordinal",
        ] {
            assert!(
                !object.contains_key(retired),
                "retired carrier {retired} is back on the serialized submit"
            );
            let mut readmitted = object.clone();
            readmitted.insert(retired.to_string(), serde_json::Value::Null);
            assert!(
                serde_json::from_value::<RenderSubmit>(serde_json::Value::Object(readmitted))
                    .is_err(),
                "a submit still carrying {retired} decoded instead of refusing"
            );
        }
    }

    #[test]
    fn political_texture_packet_requires_same_submit_static_consumer() {
        let mut frame = production_frame_with_selected_present();
        frame.submits[0]
            .political_textures
            .push(PoliticalTexturePacket {
                kind: PoliticalTextureKind::Secondary,
                target_binding_identity: "ProvinceSecondaryColorMap".into(),
                upload_order: 7,
                width: 1,
                height: 1,
                row_pitch: 4,
                format: PoliticalTextureFormat::B8g8r8a8UnormSrgb,
                bytes: vec![1, 2, 3, 4],
                sha256: "9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a".into(),
                occupation_observation: None,
            });
        let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(
            error.contains("no exact selected-submit static shader-resource consumer"),
            "{error}"
        );
    }

    #[test]
    fn derived_runtime_draw_order_reproduces_the_retired_carried_list() {
        // kills: RenderSubmit.ordered_draws used to ride the wire verbatim.
        // Capture-side promotion built that list by sorting every particle and
        // entity reference on draw_order and refusing anything that was not a
        // complete contiguous cross-family sequence, so the carried list was a
        // pure function of the recorded packets. The derivation must equal the
        // previously carried list on sealed fixtures and fail closed when the
        // recorded packets do not cover 0..n contiguously.
        let mut particle = |order: u32| ParticlePacket {
            effect_instance_id: 3,
            registration_token: "smoke".into(),
            resolved_definition_sha256: hash(b"definition"),
            content_identity: particle_content_identity(4),
            content_identity_sha256: hash(b"content"),
            bucket: 2,
            packet_ordinal: order,
            draw_order: order,
            ribbon_tail_order: None,
            canonical_packet_112_bytes: vec![0u8; 112],
            canonical_packet_sha256: hash(&[0u8; 112]),
            packet_matrix_4x4_f32_bits: [0; 16],
            live_count: 1,
            record_stride: 48,
            max_count: 1,
            ordered_gpu_instance_records: vec![0; 48],
            gpu_instance_records_sha256: hash(&[0u8; 48]),
            elapsed_f32_bits: 0,
            shader_program: shader(),
            ordered_texture_bindings: vec![texture()],
            pipeline: pipeline(),
            geometry: particle_geometry(1),
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output_state(),
            atlas_rows: 1,
            atlas_columns: 1,
            scale_f32_bits: 0,
            flags_u16: 0,
        };
        let mut entity = |order: u32| EntitySubmission {
            pass_ordinal: 1,
            draw_order: order,
            entity_instance_id: 8,
            resolved_entity_sha256: "c".repeat(64),
            content_identity: entity_content_identity(),
            shader_program: shader(),
            ordered_texture_bindings: vec![texture()],
            pipeline: pipeline(),
            geometry: entity_geometry(),
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output_state(),
            world_transform_4x4_f32_bits: [0; 16],
            state_flags: 3 | (4 << 8) | (7 << 16) | (6 << 24),
            skinning: Skinning::NotSkinned,
        };
        let mut frame = production_frame_with_selected_present();
        let submit = &mut frame.submits[0];
        submit.particles.push(particle(2));
        submit.entities.push(entity(0));
        submit.particles.push(particle(1));
        let carried_order_that_used_to_ride_the_wire = vec![
            RenderOutputRef::Entity {
                pass_ordinal: 1,
                entity_instance_id: 8,
                draw_order: 0,
            },
            RenderOutputRef::Particle {
                effect_instance_id: 3,
                bucket: 2,
                packet_ordinal: 1,
                draw_order: 1,
            },
            RenderOutputRef::Particle {
                effect_instance_id: 3,
                bucket: 2,
                packet_ordinal: 2,
                draw_order: 2,
            },
        ];
        assert_eq!(
            submit.derive_ordered_draws().unwrap(),
            carried_order_that_used_to_ride_the_wire
        );

        let mut gapped = production_frame_with_selected_present();
        let submit = &mut gapped.submits[0];
        submit.entities.push(entity(1));
        assert!(submit
            .derive_ordered_draws()
            .unwrap_err()
            .contains("contiguous"));
    }

    fn snow_mud_fade_submit(bits: u32) -> RenderSubmit {
        let mut frame = production_frame_with_selected_present();
        let mut submit = frame.submits.remove(0);
        let mut raw_bytes = vec![0u8; 256];
        raw_bytes[248..252].copy_from_slice(&bits.to_le_bytes());
        let layout = format!(
            "d3d11-cbuffer-v1[14:SceneConstants;0;256;0;{}:{}:240:16:0:3:vec]",
            SNOW_MUD_FADE_VARIABLE.len(),
            SNOW_MUD_FADE_VARIABLE,
        );
        let texture = texture();
        let mut shader = shader();
        shader.source = content("gfx/fx/pdxmap.shader");
        let draw = StaticMapDraw {
            draw_token: "snow-mud-consumer".into(),
            shader_program: shader,
            ordered_shader_resources: vec![StaticShaderResourceBinding {
                stage: ShaderStage::Pixel,
                slot: 7,
                semantic_token: SNOW_MUD_TEXTURE_SEMANTIC.into(),
                source: StaticShaderResourceSource::UsedContent {
                    content: texture.content,
                    resource: texture.resource,
                    view: texture.view,
                },
            }],
            pipeline: pipeline(),
            geometry: StaticGeometrySource::Exact {
                geometry: entity_geometry(),
            },
            ordered_constant_buffers: vec![ConstantBufferBinding {
                stage_mask_u8: 0b10,
                slot: 0,
                byte_offset: 0,
                byte_count: 256,
                sha256: hash(&raw_bytes),
                raw_bytes,
                semantic_layout: layout,
            }],
            output_state: output_state(),
            payload_sha256: "d".repeat(64),
        };
        submit.ordered_static_map_programs = vec![StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: format!("opaque-pass:{}", "e".repeat(64)),
            ordered_content_keys: vec![],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 7,
                payload_sha256: "f".repeat(64),
                command: StaticMapCommand::Draw { draw },
            }],
            payload_sha256: "1".repeat(64),
        }];
        submit
    }

    #[test]
    fn snow_mud_fade_receipt_is_reflection_bound_and_consumer_complete() {
        let selected = "a".repeat(64);
        let submit = snow_mud_fade_submit(0.25f32.to_bits());
        let receipt = selected_present_snow_mud_fade_receipt(&submit, &selected).unwrap();
        assert_eq!(receipt.fade_f32_bits, 0.25f32.to_bits());
        assert_eq!(receipt.consumer_count, 1);
        assert_eq!(receipt.consumer_set_sha256.len(), 64);

        let mut missing = submit.clone();
        let StaticMapCommand::Draw { draw } =
            &mut missing.ordered_static_map_programs[0].ordered_operations[0].command
        else {
            unreachable!()
        };
        draw.ordered_constant_buffers[0].semantic_layout =
            "d3d11-cbuffer-v1[14:SceneConstants;0;256;0]".into();
        assert!(selected_present_snow_mud_fade_receipt(&missing, &selected)
            .unwrap_err()
            .contains("lacks one exact reflected fade consumer"));

        let mut disagree = submit;
        let mut second = disagree.ordered_static_map_programs[0].ordered_operations[0].clone();
        second.operation_ordinal = 8;
        let StaticMapCommand::Draw { draw } = &mut second.command else {
            unreachable!()
        };
        draw.ordered_constant_buffers[0].raw_bytes[248..252]
            .copy_from_slice(&0.75f32.to_bits().to_le_bytes());
        draw.ordered_constant_buffers[0].sha256 = hash(&draw.ordered_constant_buffers[0].raw_bytes);
        disagree.ordered_static_map_programs[0]
            .ordered_operations
            .push(second);
        assert!(selected_present_snow_mud_fade_receipt(&disagree, &selected)
            .unwrap_err()
            .contains("disagree"));
    }

    #[test]
    fn selected_present_receipt_is_required_exact_and_tamper_evident() {
        let frame = production_frame_with_selected_present();
        frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();

        let mut zero_candidates = frame.clone();
        zero_candidates.selected_present.candidate_count = 0;
        assert!(zero_candidates
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        // The retarget case states the truth after BOTH retirements rather than a refusal that
        // no longer exists. RenderSubmit::submit_index (retired on master) was the only value
        // that could disagree with the receipt here, and the display/checkpoint identity join
        // that stood in for it went out with the retired per-frame census, so the candidate id
        // now simply IS the retained submit's identity: renaming it is a coherent frame, not a
        // tamper the wire can see. What the wire still owes is that the rename is followed
        // everywhere - the retained submit answers under the receipt's id and under no other.
        // Re-admission of the retired carried twin is caught by
        // `retired_submit_carriers_cannot_ride_the_wire`, not here.
        let mut retargeted = frame.clone();
        retargeted.selected_present.selected_candidate_id = 12;
        retargeted
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .expect("a retarget renames the identity; nothing left on the wire disagrees with it");
        assert_eq!(retargeted.retained_submit_identity(), Some(12));
        assert!(retargeted.submit(12).is_some());
        assert_eq!(retargeted.submit(11), None);

        let mut multi = frame.clone();
        multi.selected_present.candidate_count = 2;
        multi.selected_present.selected_ordinal = 1;
        multi.selected_present.candidate_set_sha256 = "d".repeat(64);
        multi.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        multi.selected_present.selected_ordinal = 0;
        assert!(multi
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut wrong_raw = frame.clone();
        wrong_raw.selected_present.selected_game_hour_raw += 1;
        assert!(wrong_raw
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut malformed_present = serde_json::to_value(&frame).unwrap();
        let canonical_candidate_id =
            malformed_present["selected_present"]["selected_candidate_id"].clone();
        malformed_present["selected_present"]["selected_candidate_id"] = serde_json::json!("011");
        assert!(
            serde_json::from_value::<ProductionRenderFrame>(malformed_present.clone())
                .unwrap_err()
                .to_string()
                .contains("u64 text is not canonical decimal")
        );
        malformed_present["selected_present"]["selected_candidate_id"] = canonical_candidate_id;
        serde_json::from_value::<ProductionRenderFrame>(malformed_present).unwrap();

        let mut unknown_present_field = serde_json::to_value(&frame).unwrap();
        unknown_present_field["selected_present"]["guessed_present_kind"] =
            serde_json::json!("display");
        assert!(
            serde_json::from_value::<ProductionRenderFrame>(unknown_present_field)
                .unwrap_err()
                .to_string()
                .contains("unknown field `guessed_present_kind`")
        );

        let mut bad_receipt = frame;
        bad_receipt.selected_present.candidate_set_sha256 = "z".repeat(64);
        assert!(bad_receipt.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err().contains("SHA-256"));
    }

    #[test]
    fn older_v3_without_optional_input_or_ui_pass_remains_compatible() {
        let frame = production_frame_with_selected_present();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();

        let value = serde_json::to_value(&frame).unwrap();
        assert!(value["selected_present"].get("recorded_input_observation").is_none());
        assert!(value.get("authenticated_ui_pass").is_none());
        assert_eq!(ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(), frame);
    }

    fn shader() -> ShaderProgram {
        let vs = vec![1, 2, 3, 4];
        let ps = vec![5, 6, 7, 8];
        ShaderProgram {
            source: content("gfx/fx/particle.shader"),
            producer_registration_root_rva: None,
            effect_token: "ParticleAdditive".into(),
            vertex_entry_token: "VertexSimpleParticle".into(),
            pixel_entry_token: "PixelSimpleParticle".into(),
            ordered_defines: vec![ShaderDefine {
                name: "PARTICLE".into(),
                value: "1".into(),
                scope: ShaderDefineScope::Effect,
            }],
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "fxc-10.1".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&vs),
                bytecode: vs,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "fxc-10.1".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&ps),
                bytecode: ps,
            },
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        }
    }

    fn constant_buffer() -> ConstantBufferBinding {
        ConstantBufferBinding {
            stage_mask_u8: 1,
            slot: 0,
            byte_offset: 0,
            byte_count: 16,
            raw_bytes: vec![0; 16],
            sha256: hash(&vec![0; 16]),
            semantic_layout: "ProjectionMatrix".into(),
        }
    }

    fn mapname_constant_buffer() -> ConstantBufferBinding {
        let mut raw_bytes = Vec::with_capacity(32);
        for bits in [0.5f32.to_bits(), (-5632.0f32).to_bits(), 0, 0, 0, 0, 0, 0] {
            raw_bytes.extend_from_slice(&bits.to_le_bytes());
        }
        ConstantBufferBinding {
            stage_mask_u8: 0b11,
            slot: 1,
            byte_offset: 0,
            byte_count: 32,
            sha256: hash(&raw_bytes),
            raw_bytes,
            semantic_layout: MAPNAME_TRANSP_OFFSET_SEMANTIC.into(),
        }
    }

    fn texture() -> TextureBinding {
        let texel = vec![255u8; 4];
        TextureBinding {
            stage: ShaderStage::Pixel,
            slot: 0,
            semantic_token: "DiffuseMap".into(),
            content: content("gfx/textures/particle.dds"),
            resource: TextureResourceDesc {
                dimension: TextureResourceDimension::Texture2d,
                width: 1,
                height: 1,
                depth: 1,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 28,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0x8,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
                ordered_subresources: vec![TextureSubresourcePayload {
                    subresource_index: 0,
                    mip_level: 0,
                    array_slice: 0,
                    row_pitch: 4,
                    slice_pitch: 4,
                    source: BufferSource::Inline {
                        sha256: hash(&texel),
                        bytes: texel,
                    },
                }],
            },
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 29,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
        }
    }

    #[test]
    fn authenticated_draw_time_texture_snapshot_is_exact_and_fail_closed() {
        let texture = texture();
        let mut packet = AuthenticatedTextureSnapshotPacket::sealed(texture.resource).unwrap();
        let mut draw = StaticMapDraw {
            draw_token: "static.generated.terrain_id".into(),
            shader_program: shader(),
            ordered_shader_resources: vec![StaticShaderResourceBinding {
                stage: texture.stage,
                slot: texture.slot,
                semantic_token: "TerrainIDMap".into(),
                source: StaticShaderResourceSource::AuthenticatedSnapshot {
                    snapshot_sha256: packet.snapshot_sha256.clone(),
                    view: texture.view,
                },
            }],
            pipeline: pipeline(),
            geometry: StaticGeometrySource::Exact {
                geometry: entity_geometry(),
            },
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output_state(),
            payload_sha256: String::new(),
        };
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        validate_static_map_draw(&draw).expect("complete authenticated snapshot");

        let mut truncated = packet.clone();
        truncated.resource.ordered_subresources.clear();
        assert!(truncated
            .validate()
            .unwrap_err()
            .contains("does not carry every ordered subresource"));

        packet.resource.ordered_subresources[0].source = BufferSource::UsedContentRange {
            content: content("gfx/terrain/terrain_id.dds"),
            source_byte_offset: 0,
            byte_count: 1,
            range_sha256: "c".repeat(64),
        };
        assert!(packet
            .validate()
            .unwrap_err()
            .contains("complete inline non-MSAA subresources"));
    }

    #[test]
    fn production_wire_base64_deduplicates_equal_inline_gpu_payloads() {
        let bytes = vec![0, 1, 2, 3, 127, 128, 254, 255];
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let inline = serde_json::json!({
            "kind": "inline",
            "bytes": base64_bytes::encode(&bytes),
            "sha256": sha256,
        });
        let mut value = serde_json::json!([inline.clone(), inline]);
        let original = value.clone();
        let mut blobs = BTreeMap::new();
        deduplicate_inline_blob_references(&mut value, &mut blobs).unwrap();
        assert_eq!(blobs.len(), 1);
        assert_ne!(value, original);
        let mut used = BTreeSet::new();
        restore_inline_blob_references(&mut value, &blobs, &mut used).unwrap();
        assert_eq!(used.len(), 1);
        assert_eq!(value, original);
    }

    fn particle_geometry(instance_count: u32) -> GeometryBinding {
        let indices = vec![0u8, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0];
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::ParticleInstanceRecords,
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&indices),
                    bytes: indices,
                },
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexedInstanced {
                index_count_per_instance: 6,
                instance_count,
                start_index: 0,
                base_vertex: 0,
                start_instance: 0,
            },
        }
    }

    fn entity_geometry() -> GeometryBinding {
        let indices = vec![0u8, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0];
        let vertices = vec![0u8; 48];
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&vertices),
                    bytes: vertices,
                },
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&indices),
                    bytes: indices,
                },
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexed {
                index_count: 6,
                start_index: 0,
                base_vertex: 0,
            },
        }
    }

    pub(crate) fn shared_test_pipeline() -> PipelineDesc {
        pipeline()
    }

    fn pipeline() -> PipelineDesc {
        PipelineDesc {
            topology: PrimitiveTopology::TriangleList,
            blend: BlendDesc {
                alpha_to_coverage_enabled: false,
                independent_blend_enabled: false,
                enabled: true,
                source_color: BlendFactor::SourceAlpha,
                destination_color: BlendFactor::One,
                color_operation: BlendOperation::Add,
                source_alpha: BlendFactor::Zero,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
                write_mask_rgba_u8: 0x7,
                additional_targets: vec![],
                blend_factor_rgba_f32_bits: [1f32.to_bits(); 4],
                sample_mask_u32: u32::MAX,
            },
            depth: DepthDesc {
                enabled: true,
                write_enabled: false,
                comparison: Comparison::LessEqual,
                stencil_enabled: false,
                stencil_read_mask_u8: 0xff,
                stencil_write_mask_u8: 0xff,
                front_face: StencilFaceDesc {
                    fail_operation: StencilOperation::Keep,
                    depth_fail_operation: StencilOperation::Keep,
                    pass_operation: StencilOperation::Keep,
                    comparison: Comparison::Always,
                },
                back_face: StencilFaceDesc {
                    fail_operation: StencilOperation::Keep,
                    depth_fail_operation: StencilOperation::Keep,
                    pass_operation: StencilOperation::Keep,
                    comparison: Comparison::Always,
                },
                stencil_reference_u32: 0,
            },
            rasterizer: RasterizerDesc {
                fill: FillMode::Solid,
                cull: CullMode::Back,
                front_counter_clockwise: false,
                depth_clip_enabled: true,
                scissor_enabled: false,
                depth_bias_i32: 0,
                depth_bias_clamp_f32_bits: 0f32.to_bits(),
                slope_scaled_depth_bias_f32_bits: 0f32.to_bits(),
                multisample_enabled: false,
                antialiased_line_enabled: false,
            },
            samplers: vec![SamplerDesc {
                stage: ShaderStage::Pixel,
                slot: 0,
                semantic_token: "DiffuseMap".into(),
                min_filter: Filter::Linear,
                mag_filter: Filter::Linear,
                mip_filter: Filter::Linear,
                address_u: AddressMode::Wrap,
                address_v: AddressMode::Wrap,
                address_w: AddressMode::Wrap,
                comparison: None,
                mip_lod_bias_f32_bits: 0f32.to_bits(),
                max_anisotropy: 1,
                border_color_rgba_f32_bits: [0f32.to_bits(); 4],
                min_lod_f32_bits: 0f32.to_bits(),
                max_lod_f32_bits: f32::MAX.to_bits(),
            }],
            input_layout: vec![InputElementDesc {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                dxgi_format_u32: 6,
                input_slot: 0,
                aligned_byte_offset: 0,
                per_instance: true,
                instance_step_rate: 1,
            }],
            index_format: IndexFormat::R16Uint,
            static_indices_u16: vec![0, 1, 2, 2, 1, 3],
        }
    }

    fn output_state() -> RenderOutputState {
        RenderOutputState {
            ordered_color_targets: vec![RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: 0,
                },
                width: 32,
                height: 32,
                dxgi_format_u32: 10,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 32f32.to_bits(),
                height_f32_bits: 32f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        }
    }

    fn identity_transform() -> [u32; 16] {
        [
            1.0f32.to_bits(),
            0,
            0,
            0,
            0,
            1.0f32.to_bits(),
            0,
            0,
            0,
            0,
            1.0f32.to_bits(),
            0,
            10.0f32.to_bits(),
            20.0f32.to_bits(),
            30.0f32.to_bits(),
            1.0f32.to_bits(),
        ]
    }

    fn production_frame_with_o04() -> ProductionRenderFrame {
        let mut frame = production_frame_with_selected_present();
        frame.submits[0].ordered_render_operations.clear();
        let output = RenderOutputRef::Entity {
            pass_ordinal: 1,
            entity_instance_id: 8,
            draw_order: 0,
        };
        frame.submits[0].entities = vec![EntitySubmission {
            pass_ordinal: 1,
            draw_order: 0,
            entity_instance_id: 8,
            resolved_entity_sha256: "c".repeat(64),
            content_identity: entity_content_identity(),
            shader_program: shader(),
            ordered_texture_bindings: vec![texture()],
            pipeline: pipeline(),
            geometry: entity_geometry(),
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output_state(),
            world_transform_4x4_f32_bits: identity_transform(),
            state_flags: 3 | (4 << 8) | (7 << 16) | (6 << 24),
            skinning: Skinning::NotSkinned,
        }];
        frame.o04_cities = Some(O04CityFrame {
            schema: O04_CITY_SCHEMA.into(),
            executable_sha256: O04_A729_EXECUTABLE_SHA256.into(),
            evidence_manifest_sha256: O04_EVIDENCE_MANIFEST_SHA256.into(),
            live_city_submit_binding_sha256: O04_LIVE_CITY_SUBMIT_BINDING_SHA256.into(),
            live_city_submit_verification_sha256: O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            cities: vec![O04CitySemantic {
                event_sequence: frame.last_sequence,
                generator_ordinal: 73,
                candidate_xy: [12, 34],
                province_id: 8819,
                color_index: 7,
                density_f32_bits: 0.75f32.to_bits(),
                entity_scale_f32_bits: 1.25f32.to_bits(),
                entity_asset: entity_content_identity().resolved_entity_asset,
                mesh: entity_content_identity().mesh,
                world_transform_4x4_f32_bits: identity_transform(),
                citylight_state: O04CitylightState::RegisteredCitylight,
                referenced_outputs: vec![output.clone()],
            }],
            ordered_outputs: vec![output],
        });
        frame
    }

    const EXACT_UI_PASS_DRAW_COUNT: u32 = 1167;

    fn production_frame_with_exact_ui_pass() -> ProductionRenderFrame {
        let mut frame = production_frame_with_selected_present();
        let mut input_observation = RecordedInputObservationReceipt {
            executable_sha256: UI_A729_EXECUTABLE_SHA256.into(),
            process_id: 7,
            process_start_nonce: 8,
            hook_epoch_nonce: 11,
            observation_sequence: 1,
            prior_observation_qpc_ticks: frame.selected_present.selected_qpc_ticks - 3,
            selected_present_qpc_ticks: frame.selected_present.selected_qpc_ticks,
            present_sample_started_qpc_ticks: frame.selected_present.selected_qpc_ticks + 1,
            present_sample_completed_qpc_ticks: frame.selected_present.selected_qpc_ticks + 2,
            selected_present_receipt_sha256: frame.selected_present.computed_sha256().unwrap(),
            client_extent: [1920, 1080],
            prior_client_position: [10, 20],
            current_client_position: [11, 20],
            variant: RecordedInputVariant::MouseKeyboard,
            authority_manifest_sha256: RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256.into(),
            observation_receipt_sha256: String::new(),
        };
        input_observation.observation_receipt_sha256 =
            input_observation.computed_receipt_sha256().unwrap();
        frame.selected_present.recorded_input_observation = Some(input_observation);
        let pipeline_draw = exact_hud_pipeline_draw();
        let target = pipeline_draw.output_state.ordered_color_targets[0].clone();
        let mut ui_pass_operations = Vec::new();
        let gui_definition = content("interface/modded_selected_hud.gui");
        for ui_pass_ordinal in 0..EXACT_UI_PASS_DRAW_COUNT {
            let identity = HudCommandIdentity {
                    authored_surface_identity: "modded_selected_present_hud".into(),
                    runtime_root: "modded_selected_present_root".into(),
                    selected_gui_definition: gui_definition.clone(),
                    engine_instance_token: u64::from(ui_pass_ordinal) + 1,
                    child_path: vec![ui_pass_ordinal],
                    element_type: "modded_paint_element".into(),
                    name: format!("element-{ui_pass_ordinal}"),
                    paint_ordinal: ui_pass_ordinal,
                };
                let placement = HudPlacement {
                    destination_rect: if ui_pass_ordinal == 0 {
                        UiPassRect {
                            left_i32: 0,
                            top_i32: 0,
                            right_i32: 32,
                            bottom_i32: 32,
                        }
                    } else {
                        UiPassRect {
                            left_i32: 0,
                            top_i32: 0,
                            right_i32: 1,
                            bottom_i32: 1,
                        }
                    },
                    clip_state: UiPassClip::Disabled,
                    surface_paint_ordinal: ui_pass_ordinal,
                };
                let mut ordered_content_for_paint = Vec::new();
                let execution = if ui_pass_ordinal == 0 {
                    HudCommandExecution::ExactPipeline {
                        draw: UiExactPipelineDraw {
                            exact_draw: pipeline_draw.clone(),
                        },
                    }
                } else {
                    let (paint, extra_content) = match ui_pass_ordinal % 3 {
                        0 => {
                            let law = match ui_pass_ordinal % 5 {
                                0 => SpriteLaw::Plain,
                                1 => SpriteLaw::Scaled,
                                2 => SpriteLaw::NineSlice {
                                    border_x: 2,
                                    border_y: 3,
                                },
                                3 => SpriteLaw::ProgressLeftCrop {
                                    state_bits: 0.5f32.to_bits(),
                                },
                                _ => SpriteLaw::ProgressMinMax {
                                    state_bits: 0.75f32.to_bits(),
                                },
                            };
                            let is_minmax = matches!(law, SpriteLaw::ProgressMinMax { .. });
                            let mut content_rows = vec![
                                content("interface/modded_selected_hud.gfx"),
                                content("gfx/interface/modded_selected_hud.dds"),
                            ];
                            if is_minmax {
                                content_rows
                                    .push(content("gfx/interface/modded_selected_hud_back.dds"));
                            }
                            (
                                HudPaintCommand::Sprite {
                                    sprite_id: format!("sprite-{ui_pass_ordinal}"),
                                    resolved_frame: ui_pass_ordinal % 4,
                                    law,
                                    sprite_definition_content_index: 1,
                                    texture_content_index: 2,
                                    back_texture_content_index: is_minmax.then_some(3),
                                },
                                content_rows,
                            )
                        }
                        1 => (
                            HudPaintCommand::Text {
                                runtime_text_context_id: format!("runtime-text-{ui_pass_ordinal}"),
                                runtime_text_frame_hour: frame.update_index as i64,
                                runtime_text_pov_country_or_global: "MOD".into(),
                                runtime_text_view_id: "modded_selected_present_root".into(),
                                runtime_text_element_id: format!("element-{ui_pass_ordinal}"),
                                rgba: [248, 249, 250, 255],
                                horizontal_alignment: TextHorizontalAlignment::Center,
                                layout_inputs: TextLayoutInputs::AuthoredReproducible {
                                    authored_rect_offsets: TextRectOffsets {
                                        left_i32: 0,
                                        top_i32: 0,
                                        right_i32: 160,
                                        bottom_i32: 24,
                                    },
                                    max_width: Some(160),
                                    wrap: true,
                                    clip_rect_offsets: Some(TextRectOffsets {
                                        left_i32: 0,
                                        top_i32: 0,
                                        right_i32: 160,
                                        bottom_i32: 24,
                                    }),
                                },
                                font_bindings: vec![
                                    FontBinding {
                                        font_identity: "modded_selected_hud".into(),
                                        definition_content_index: 1,
                                        atlas_texture_content_indices: vec![2],
                                    },
                                    FontBinding {
                                        font_identity: "modded_selected_hud_fallback".into(),
                                        definition_content_index: 3,
                                        atlas_texture_content_indices: vec![4, 5],
                                    },
                                ],
                                inline_icon_bindings: vec![InlineIconBinding {
                                    sprite_identity: "GFX_modded_selected_hud_inline".into(),
                                    definition_content_index: 6,
                                    texture_content_indices: vec![7],
                                }],
                            },
                            vec![
                                content("interface/fonts/modded_selected_hud.font"),
                                content("gfx/fonts/modded_selected_hud.dds"),
                                content("interface/fonts/modded_selected_hud_fallback.font"),
                                content("gfx/fonts/modded_selected_hud_fallback_0.dds"),
                                content("gfx/fonts/modded_selected_hud_fallback_1.dds"),
                                content("interface/modded_selected_hud_inline.gfx"),
                                content("gfx/interface/modded_selected_hud_inline.dds"),
                            ],
                        ),
                        _ => (
                            HudPaintCommand::MaskedSprite {
                                sprite_id: format!("masked-sprite-{ui_pass_ordinal}"),
                                mask_slot: 1,
                                overlay_slot: 2,
                                sprite_definition_content_index: 1,
                                texture_content_index: 2,
                                mask_texture_content_index: 3,
                                overlay_texture_content_index: 4,
                            },
                            vec![
                                content("interface/modded_masked_hud.gfx"),
                                content("gfx/interface/modded_masked_hud.dds"),
                                content("gfx/interface/modded_mask.dds"),
                                content("gfx/interface/modded_overlay.dds"),
                            ],
                        ),
                    };
                    ordered_content_for_paint = extra_content;
                    HudCommandExecution::InterpretedPaint { paint }
                };
                let mut ordered_content = vec![gui_definition.clone()];
                if let HudCommandExecution::ExactPipeline { draw } = &execution {
                    for content in collect_static_draw_used_content(&draw.exact_draw) {
                        if !ordered_content.contains(&content) {
                            ordered_content.push(content);
                        }
                    }
                } else {
                    ordered_content.extend(ordered_content_for_paint);
                }
                let mut command = HudCommand {
                    ordinal: ui_pass_ordinal,
                    identity,
                    placement,
                    execution,
                    ordered_effective_vfs_used_content: ordered_content,
                    command_identity_sha256: String::new(),
                };
                command.command_identity_sha256 =
                    command.computed_identity_sha256(&target).unwrap();
                ui_pass_operations.push(UiPassOperation::HudCommand { command });
        }
        let mut authenticated_ui_pass = AuthenticatedUiPass {
            schema: UI_PASS_SCHEMA.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            ui_pass_draw_count: EXACT_UI_PASS_DRAW_COUNT,
            target,
            selected_present_receipt_sha256: frame.selected_present.computed_sha256().unwrap(),
            producer_selection_receipt_sha256: "7".repeat(64),
            source_frontier_sha256: "8".repeat(64),
            recorded_input_receipt_sha256: frame
                .selected_present
                .recorded_input_observation
                .as_ref()
                .unwrap()
                .observation_receipt_sha256
                .clone(),
            active_mapmode_state: Some(ActiveMapmodeState {
                mode_token: "modded_weather_mapmode".into(),
                button_ordinal: 7,
                authored_button_identity: "modded_weather_button".into(),
                layout_root_token: "modded_8_button_mapmode".into(),
                provenance: MapmodeProvenance::EffectiveVfs {
                    content: gui_definition,
                },
            }),
            ui_pass_operations,
            pass_identity_sha256: String::new(),
        };
        authenticated_ui_pass.pass_identity_sha256 =
            authenticated_ui_pass.computed_identity_sha256().unwrap();
        frame.authenticated_ui_pass = Some(authenticated_ui_pass);
        frame
    }

    fn exact_ui_command_mut(frame: &mut ProductionRenderFrame, ordinal: usize) -> &mut HudCommand {
        match &mut frame
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[ordinal]
        {
            UiPassOperation::HudCommand { command } => command,
        }
    }

    fn rehash_exact_ui_command_and_pass(frame: &mut ProductionRenderFrame, ordinal: usize) {
        let target = frame.authenticated_ui_pass.as_ref().unwrap().target.clone();
        let command = exact_ui_command_mut(frame, ordinal);
        command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        let ui_pass = frame.authenticated_ui_pass.as_mut().unwrap();
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
    }

    fn production_frame_with_m03_a48_row() -> ProductionRenderFrame {
        let mut frame = production_frame_with_selected_present();
        let mut draw = exact_hud_pipeline_draw();
        draw.draw_token = "m03.exact-a48-movement".into();
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        let mut used_content = collect_static_draw_used_content(&draw);
        used_content.sort();
        let stable_key = M03StableKey::Movement {
            owner_country_id: 1,
            division_id: 42,
            movement_generation: 3,
        };
        let draw_ref = M03ArrowDrawRef {
            engine_draw_ordinal: 23,
        };
        frame.m03_arrows = Some(M03ArrowFrame {
            schema: M03_ARROW_SCHEMA.into(),
            executable_sha256: M03_A729_EXECUTABLE_SHA256.into(),
            static_semantic_proof_sha256: M03_STATIC_PROOF_SHA256.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            used_content,
            rows: vec![M03ArrowRow {
                event_sequence: frame.last_sequence,
                stable_key: stable_key.clone(),
                definition_name: "moveArrow".into(),
                plan_subtype: None,
                ordered_division_ids: vec![],
                ordered_fixed_point_input_points_1e5: vec![[0, 0], [100_000, 100_000]],
                ordered_engine_working_points_1e5: Some(vec![[0, 0], [100_000, 100_000]]),
                assigned: true,
                active: true,
                selected: false,
                progress_raw_u32: 5,
                fade_in_f32_bits: 0.2f32.to_bits(),
                fade_out_f32_bits: 0.0f32.to_bits(),
                culling_flags_u16: 0,
                visibility: M03Visibility::Submitted,
                manager_insertion_ordinal: 4,
                definition_group_ordinal: 2,
                comparator_definition_a0_f32_bits: 2.0f32.to_bits(),
                comparator_definition_a4_f32_bits: 3.0f32.to_bits(),
                comparator_definition_c8_f32_bits: 0.5f32.to_bits(),
                comparator_arrow_178_f32_bits: 4.0f32.to_bits(),
                comparator_primary_f32_bits: 5.0f32.to_bits(),
                comparator_tie_17c_f32_bits: 1.0f32.to_bits(),
                draw_members: vec![M03ArrowDrawMemberRef {
                    engine_draw_ordinal: 23,
                    member_ordinal: 0,
                }],
            }],
            draws: vec![M03ArrowDraw {
                engine_draw_ordinal: 23,
                draw,
                members: vec![M03ArrowDrawMember {
                    stable_key,
                    member_ordinal: 0,
                    first_vertex: 0,
                    vertex_count: 6,
                }],
            }],
            ordered_draws: vec![draw_ref.clone()],
        });
        frame.submits[0].ordered_render_operations =
            vec![RenderExecutionRef::M03 { output: draw_ref }];
        frame
    }

    #[test]
    fn o04_semantic_carrier_round_trips_and_rejects_join_vfs_and_evidence_tamper() {
        let frame = production_frame_with_o04();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(), frame);

        let mut dangling = frame.clone();
        if let Some(RenderOutputRef::Entity { draw_order, .. }) =
            dangling.o04_cities.as_mut().unwrap().cities[0]
                .referenced_outputs
                .first_mut()
        {
            *draw_order = 99;
        }
        assert!(dangling.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err().contains("ordered outputs"));

        let mut mismatched_transform = frame.clone();
        mismatched_transform.o04_cities.as_mut().unwrap().cities[0].world_transform_4x4_f32_bits
            [12] = 11.0f32.to_bits();
        assert!(mismatched_transform
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("differs from entity submission"));

        let mut wrong_vfs = frame.clone();
        wrong_vfs.o04_cities.as_mut().unwrap().cities[0]
            .entity_asset
            .effective_vfs_manifest_sha256 = "f".repeat(64);
        assert!(wrong_vfs
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("content authority"));

        let mut wrong_evidence = frame;
        wrong_evidence
            .o04_cities
            .as_mut()
            .unwrap()
            .evidence_manifest_sha256 = "e".repeat(64);
        assert!(wrong_evidence
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("evidence authority"));
    }

    #[test]
    fn exact_ui_pass_round_trips_and_rejects_gap_ref_ordinal_and_target_tamper() {
        let frame = production_frame_with_exact_ui_pass();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(), frame);

        let mut gap = frame.clone();
        gap.authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations
            .pop();
        assert!(gap.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err().contains("draw frontier"));

        let mut wrong_ref = frame.clone();
        {
            // HUD-command-only operation set; see UiPassOperation. The block
            // keeps the mutable borrow scoped exactly as the `if let` did.
            let UiPassOperation::HudCommand { command } = &mut wrong_ref
                .authenticated_ui_pass
                .as_mut()
                .unwrap()
                .ui_pass_operations[859];
            command.ordinal = 858;
        }
        assert!(wrong_ref
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("not exact 0..N-1 order"));

        let mut wrong_target = frame.clone();
        {
            let UiPassOperation::HudCommand { command } = &mut wrong_target
                .authenticated_ui_pass
                .as_mut()
                .unwrap()
                .ui_pass_operations[0];
            command.placement.destination_rect.right_i32 = 31;
        }
        assert!(wrong_target
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("identity SHA-256 mismatch"));

        let mut duplicate_ref = frame;
        let mut duplicate = match &duplicate_ref
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .ui_pass_operations[1]
        {
            UiPassOperation::HudCommand { command } => command.clone(),
        };
        let duplicate_target = duplicate_ref
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        duplicate.ordinal = 0;
        duplicate.command_identity_sha256 = duplicate
            .computed_identity_sha256(&duplicate_target)
            .unwrap();
        duplicate_ref
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[0] = UiPassOperation::HudCommand { command: duplicate };
        assert!(duplicate_ref
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("identity is duplicated"));

        let mut incomplete_content_join = production_frame_with_exact_ui_pass();
        let target = incomplete_content_join
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = incomplete_content_join
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        {
            let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[1];
            command.ordered_effective_vfs_used_content.pop();
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(incomplete_content_join
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("content index is invalid"));

        let mut missing_minmax_back = production_frame_with_exact_ui_pass();
        let target = missing_minmax_back
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = missing_minmax_back.authenticated_ui_pass.as_mut().unwrap();
        {
            let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[9];
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = None;
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(missing_minmax_back
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("lacks exact back-texture"));

        let mut duplicate_minmax_index = production_frame_with_exact_ui_pass();
        let target = duplicate_minmax_index
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = duplicate_minmax_index
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        {
            let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[9];
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        texture_content_index,
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = Some(*texture_content_index);
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(duplicate_minmax_index
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("indices are not distinct"));

        let mut unproven_back = production_frame_with_exact_ui_pass();
        let target = unproven_back
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = unproven_back.authenticated_ui_pass.as_mut().unwrap();
        {
            let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[3];
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = Some(2);
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(unproven_back
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("unproven back texture"));

        let mut absent_input = production_frame_with_exact_ui_pass();
        absent_input.selected_present.recorded_input_observation = None;
        assert!(absent_input
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("recorded input variant"));

        let mut backdated_input = production_frame_with_exact_ui_pass();
        let observation = backdated_input
            .selected_present
            .recorded_input_observation
            .as_mut()
            .unwrap();
        observation.present_sample_started_qpc_ticks =
            observation.selected_present_qpc_ticks.saturating_sub(1);
        observation.observation_receipt_sha256 = observation.computed_receipt_sha256().unwrap();
        assert!(backdated_input
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("non-chronological"));

        // kills: carrier.production_frame.authority.recorded_input_observation — an
        // observation bound to a DIFFERENT hour's selected Present (the once-per-session
        // producer defect) must refuse on the per-frame qpc-ticks EQUALITY even while
        // its own receipt chain stays internally consistent AND its present sample
        // still completes before this frame's cutoff (fixture: selected 500, cutoff
        // 600; the foreign hour's sample ends at 402) — so the refusal can only come
        // from the equality clause, not the cutoff clause.
        //
        // The observation rides on SelectedPresentReceipt now, and it is the FIRST
        // gate in validate() - ahead of every UI-pass join - so the clause named
        // below is the one that refuses and the joins the mutation disturbs
        // downstream are unreachable. The control on the line after the fixture is
        // what makes that ordering argument load-bearing rather than assumed.
        let mut foreign_present_hour = production_frame_with_exact_ui_pass();
        foreign_present_hour
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .expect("the fixture is lawful before its observation is moved to another hour");
        let observation = foreign_present_hour
            .selected_present
            .recorded_input_observation
            .as_mut()
            .unwrap();
        assert_eq!(observation.selected_present_qpc_ticks, 500);
        assert_eq!(observation.present_sample_completed_qpc_ticks, 502);
        observation.prior_observation_qpc_ticks = 100;
        observation.selected_present_qpc_ticks = 400;
        observation.present_sample_started_qpc_ticks = 401;
        observation.present_sample_completed_qpc_ticks = 402;
        observation.observation_receipt_sha256 = observation.computed_receipt_sha256().unwrap();
        assert!(foreign_present_hour
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("crosses selected-Present cutoff"));

        // ... and so must an observation whose receipt hash uses the WRONG
        // CONSTRUCTION: the raw producer selection receipt is a binary v7-domain
        // SHA-256, this join demands the portable canonical-JSON construction, and the
        // two agree only by collision.
        let mut wrong_construction = production_frame_with_exact_ui_pass();
        wrong_construction
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .expect("the fixture is lawful before the receipt construction is forged");
        let observation = wrong_construction
            .selected_present
            .recorded_input_observation
            .as_mut()
            .unwrap();
        let mut forged = observation.selected_present_receipt_sha256.clone();
        forged
            .replace_range(..1, if forged.starts_with('0') { "1" } else { "0" });
        observation.selected_present_receipt_sha256 = forged;
        observation.observation_receipt_sha256 = observation.computed_receipt_sha256().unwrap();
        assert!(wrong_construction
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("crosses selected-Present cutoff"));

        let mut wrong_mapmode_vfs = production_frame_with_exact_ui_pass();
        if let Some(ActiveMapmodeState {
            provenance: MapmodeProvenance::EffectiveVfs { content },
            ..
        }) = &mut wrong_mapmode_vfs
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .active_mapmode_state
        {
            content.effective_vfs_manifest_sha256 = "f".repeat(64);
        }
        let ui_pass = wrong_mapmode_vfs.authenticated_ui_pass.as_mut().unwrap();
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(wrong_mapmode_vfs
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("mapmode VFS authority"));
    }

    #[test]
    fn exact_ui_pass_accepts_modded_dynamic_frontier_and_rejects_reordering() {
        // A MODDED frontier is a different draw count, not the fixture's own.
        // Master proved that through `reshape_exact_ui_m02_interleave`, which
        // truncated the pass to 23 operations and rehashed it while injecting an
        // M02 operation; the injection went with the retired carrier, the
        // reshape did not, so it is done directly here (codex round 7).
        const MODDED_FRONTIER: u32 = 23;
        let mut frame = production_frame_with_exact_ui_pass();
        assert_ne!(
            EXACT_UI_PASS_DRAW_COUNT, MODDED_FRONTIER,
            "the modded frontier must differ from the fixture's own or this proves nothing"
        );
        let pass = frame.authenticated_ui_pass.as_mut().unwrap();
        pass.ui_pass_draw_count = MODDED_FRONTIER;
        pass.ui_pass_operations.truncate(MODDED_FRONTIER as usize);
        pass.pass_identity_sha256 = pass.computed_identity_sha256().unwrap();
        frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(
            ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap(),
            frame
        );

        // kills: accepting a draw count that disagrees with the operations that
        // cover it. Master reached this through the M02 carrier's own
        // `ui_pass_draw_count`; the pass-level law is the one that survives.
        let mut wrong_frontier = frame.clone();
        let pass = wrong_frontier.authenticated_ui_pass.as_mut().unwrap();
        pass.ui_pass_draw_count = MODDED_FRONTIER + 1;
        pass.pass_identity_sha256 = pass.computed_identity_sha256().unwrap();
        assert!(wrong_frontier
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("do not exactly cover the draw frontier"));

        let mut wrong_dynamic_ordinal = frame;
        let pass = wrong_dynamic_ordinal
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        pass.ui_pass_operations.swap(16, 17);
        pass.pass_identity_sha256 = pass.computed_identity_sha256().unwrap();
        assert!(wrong_dynamic_ordinal
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("0..N-1"));
    }

    #[test]
    fn exact_ui_text_round_trips_fallback_fonts_icons_and_captured_owner_layout() {
        let frame = production_frame_with_exact_ui_pass();
        let command = match &frame
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .ui_pass_operations[1]
        {
            UiPassOperation::HudCommand { command } => command,
        };
        let HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    font_bindings,
                    inline_icon_bindings,
                    ..
                },
        } = &command.execution
        else {
            panic!("expected text paint")
        };
        assert_eq!(font_bindings.len(), 2);
        assert_eq!(font_bindings[1].atlas_texture_content_indices, vec![4, 5]);
        assert_eq!(inline_icon_bindings.len(), 1);
        assert_eq!(inline_icon_bindings[0].texture_content_indices, vec![7]);
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(), frame);

        let mut captured = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut captured, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { layout_inputs, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        *layout_inputs = TextLayoutInputs::CapturedRuntime;
        rehash_exact_ui_command_and_pass(&mut captured, 1);
        captured.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        let encoded = captured.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(
            ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(),
            captured
        );
    }

    #[test]
    fn exact_ui_text_hashes_all_inputs_and_rejects_invalid_bindings() {
        for identity_axis in ["hour", "pov", "view", "element"] {
            let mut tampered = production_frame_with_exact_ui_pass();
            let command = exact_ui_command_mut(&mut tampered, 1);
            let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Text {
                        runtime_text_frame_hour,
                        runtime_text_pov_country_or_global,
                        runtime_text_view_id,
                        runtime_text_element_id,
                        ..
                    },
            } = &mut command.execution
            else {
                unreachable!()
            };
            match identity_axis {
                "hour" => *runtime_text_frame_hour += 1,
                "pov" => runtime_text_pov_country_or_global.push_str("_tampered"),
                "view" => runtime_text_view_id.push_str("_tampered"),
                "element" => runtime_text_element_id.push_str("_tampered"),
                _ => unreachable!(),
            }
            assert!(tampered
                .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err()
                .contains("command identity SHA-256 mismatch"));
        }

        let mut rgba_tamper = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut rgba_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { rgba, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        rgba[0] ^= 1;
        assert!(rgba_tamper
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("command identity SHA-256 mismatch"));

        let mut layout_tamper = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut layout_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { layout_inputs, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        let TextLayoutInputs::AuthoredReproducible { wrap, .. } = layout_inputs else {
            unreachable!()
        };
        *wrap = !*wrap;
        assert!(layout_tamper
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("command identity SHA-256 mismatch"));

        let mut pass_hash_tamper = production_frame_with_exact_ui_pass();
        let target = pass_hash_tamper
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let command = exact_ui_command_mut(&mut pass_hash_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[0].font_identity.push_str("-tampered");
        command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        assert!(pass_hash_tamper
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("UI-pass identity SHA-256 mismatch"));

        let mut duplicate_font = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut duplicate_font, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[1].font_identity = font_bindings[0].font_identity.clone();
        rehash_exact_ui_command_and_pass(&mut duplicate_font, 1);
        assert!(duplicate_font
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("font binding identity/atlas list is invalid"));

        let mut duplicate_index = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut duplicate_index, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[1].definition_content_index = 2;
        rehash_exact_ui_command_and_pass(&mut duplicate_index, 1);
        assert!(duplicate_index
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("content indices are not distinct"));

        let mut missing_font = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut missing_font, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings.clear();
        rehash_exact_ui_command_and_pass(&mut missing_font, 1);
        assert!(missing_font
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("no exact font binding"));

        let mut missing_icon = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut missing_icon, 1);
        let HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    inline_icon_bindings,
                    ..
                },
        } = &mut command.execution
        else {
            unreachable!()
        };
        inline_icon_bindings.clear();
        rehash_exact_ui_command_and_pass(&mut missing_icon, 1);
        assert!(missing_icon
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("content order/cover is not exact"));
    }

    #[test]
    fn m03_a48_portable_row_round_trips_and_rejects_join_proof_and_vfs_tamper() {
        let frame = production_frame_with_m03_a48_row();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(), frame);

        let mut wrong_join = frame.clone();
        wrong_join.m03_arrows.as_mut().unwrap().draws[0].members[0].member_ordinal = 1;
        assert!(wrong_join
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("draw-member order/range/key"));

        let mut wrong_proof = frame.clone();
        wrong_proof
            .m03_arrows
            .as_mut()
            .unwrap()
            .static_semantic_proof_sha256 = "e".repeat(64);
        assert!(wrong_proof
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("promoted a729 static proof"));

        let mut wrong_vfs = frame.clone();
        wrong_vfs.m03_arrows.as_mut().unwrap().used_content[0].effective_vfs_manifest_sha256 =
            "f".repeat(64);
        assert!(wrong_vfs
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("content authority"));

        let mut nested_wrong_vfs = frame;
        let draw = &mut nested_wrong_vfs.m03_arrows.as_mut().unwrap().draws[0].draw;
        draw.shader_program.source.effective_vfs_manifest_sha256 = "f".repeat(64);
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        assert!(nested_wrong_vfs
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("exact cover"));
    }

    fn canonical_particle_packet(
        matrix: [u32; 16],
        live_count: u32,
        max_count: u32,
        elapsed_f32_bits: u32,
        flags_u16: u16,
        packet_ordinal: u32,
    ) -> Vec<u8> {
        let mut packet = vec![0; 112];
        for (dst, bits) in packet[..0x40].chunks_exact_mut(4).zip(matrix) {
            dst.copy_from_slice(&bits.to_le_bytes());
        }
        packet[0x40..0x44].copy_from_slice(&live_count.to_le_bytes());
        packet[0x50..0x54].copy_from_slice(&48u32.to_le_bytes());
        packet[0x54..0x58].copy_from_slice(&max_count.to_le_bytes());
        packet[0x58..0x5c].copy_from_slice(&elapsed_f32_bits.to_le_bytes());
        packet[0x5c..0x5e].copy_from_slice(&flags_u16.to_le_bytes());
        packet[0x60..0x64].copy_from_slice(&packet_ordinal.to_le_bytes());
        packet
    }

    #[test]
    fn exact_packet_and_palette_round_trip() {
        let frame = production_frame_with_static_entities_and_particles();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        let decoded = ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();
        assert_eq!(decoded, frame);
        exact_packet_and_palette_round_trip_tail(decoded, encoded);
    }

    fn production_frame_with_static_entities_and_particles() -> ProductionRenderFrame {
        let palette = vec![7; 3200];
        let canonical_packet = canonical_particle_packet([0; 16], 2, 4, 0, 0, 4);
        let mut frame = ProductionRenderFrame {
            schema: SCHEMA.into(),
            update_index: 16_959_360,
            first_sequence: 40,
            last_sequence: 42,
            presentation_qpc_frequency_hz: 10_000_000,
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                11,
                0,
                60_759_359,
                500,
                600,
            ),
            snow_mud_texture: None,
            submits: vec![RenderSubmit {
                global_order: 1,
                camera_carrier: None,
                ordered_static_map_programs: vec![],
                authenticated_texture_snapshots: vec![],
                s1_shadow: None,
                particle_packet_set_sha256: "5".repeat(64),
                particles: vec![ParticlePacket {
                    effect_instance_id: 3,
                    registration_token: "smoke".into(),
                    resolved_definition_sha256: "c".repeat(64),
                    content_identity: particle_content_identity(4),
                    content_identity_sha256: particle_content_identity(4)
                        .computed_sha256()
                        .unwrap(),
                    bucket: 2,
                    packet_ordinal: 4,
                    draw_order: 1,
                    ribbon_tail_order: None,
                    canonical_packet_112_bytes: canonical_packet.clone(),
                    canonical_packet_sha256: hash(&canonical_packet),
                    packet_matrix_4x4_f32_bits: [0; 16],
                    live_count: 2,
                    record_stride: 48,
                    max_count: 4,
                    ordered_gpu_instance_records: vec![0; 96],
                    gpu_instance_records_sha256: hash(&vec![0; 96]),
                    elapsed_f32_bits: 0,
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: particle_geometry(2),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    atlas_rows: 1,
                    atlas_columns: 1,
                    scale_f32_bits: 0,
                    flags_u16: 0,
                }],
                entities: vec![EntitySubmission {
                    pass_ordinal: 1,
                    draw_order: 0,
                    entity_instance_id: 8,
                    resolved_entity_sha256: "c".repeat(64),
                    content_identity: entity_content_identity(),
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: entity_geometry(),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    world_transform_4x4_f32_bits: [0; 16],
                    state_flags: 3 | (4 << 8) | (7 << 16) | (6 << 24),
                    skinning: Skinning::Palette {
                        palette_sha256: hash(&palette),
                        palette_3200_bytes: palette,
                    },
                }],
                rail_construction_arrows: vec![],
                ordered_render_operations: Vec::new(),
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m03_arrows: None,
            rail_supply_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            authenticated_ui_pass: None,
        };
        let gs = vec![9, 10, 11, 12];
        frame.submits[0].particles[0].shader_program.geometry_shader =
            Some(AdditionalShaderStage {
                entry_token: "GeometryParticle".into(),
                compiled: CompiledShaderPayload {
                    compiler_identity: "fxc-10.1".into(),
                    target_profile: "gs_5_0".into(),
                    compiler_flags_u64: 0,
                    sha256: hash(&gs),
                    bytecode: gs,
                },
            });
        frame.submits[0].particles[0].geometry.stream_output_targets =
            vec![StreamOutputTargetBinding {
                slot: 2,
                buffer_identity: "particle-stream-output-2".into(),
                buffer_byte_width: 4096,
                byte_offset: u32::MAX,
            }];
        let static_texture = texture();
        let mut static_program = StaticMapPassProgram {
            program_ordinal: 7,
            pass_token: format!("opaque-pass:{}", "b".repeat(64)),
            ordered_content_keys: vec![
                "gfx/fx/particle.shader".into(),
                "gfx/textures/particle.dds".into(),
            ],
            ordered_operations: vec![
                StaticMapOperation {
                    operation_ordinal: 100,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::ClearColor {
                        target: output_state().ordered_color_targets[0].clone(),
                        rgba_f32_bits: [0; 4],
                    },
                },
                StaticMapOperation {
                    operation_ordinal: 101,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::Draw {
                        draw: StaticMapDraw {
                            draw_token: "terrain-chunk-0".into(),
                            shader_program: shader(),
                            ordered_shader_resources: vec![StaticShaderResourceBinding {
                                stage: static_texture.stage,
                                slot: static_texture.slot,
                                semantic_token: static_texture.semantic_token,
                                source: StaticShaderResourceSource::UsedContent {
                                    content: static_texture.content,
                                    resource: static_texture.resource,
                                    view: static_texture.view,
                                },
                            }],
                            pipeline: pipeline(),
                            geometry: StaticGeometrySource::Exact {
                                geometry: entity_geometry(),
                            },
                            ordered_constant_buffers: vec![constant_buffer()],
                            output_state: output_state(),
                            payload_sha256: String::new(),
                        },
                    },
                },
            ],
            payload_sha256: String::new(),
        };
        static_program.seal_payload_hashes().unwrap();
        frame.submits[0].ordered_static_map_programs = vec![static_program];
        frame.submits[0].ordered_render_operations = vec![
            RenderExecutionRef::Static {
                operation_ordinal: 100,
            },
            RenderExecutionRef::Static {
                operation_ordinal: 101,
            },
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Entity {
                    pass_ordinal: 1,
                    entity_instance_id: 8,
                    draw_order: 0,
                },
            },
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Particle {
                    effect_instance_id: 3,
                    bucket: 2,
                    packet_ordinal: 4,
                    draw_order: 1,
                },
            },
        ];
        frame
    }

    fn exact_packet_and_palette_round_trip_tail(decoded: ProductionRenderFrame, encoded: Vec<u8>) {
        let mut legacy_mapname = decoded.clone();
        let StaticMapCommand::Draw { draw } =
            &mut legacy_mapname.submits[0].ordered_static_map_programs[0].ordered_operations[1]
                .command
        else {
            unreachable!()
        };
        draw.shader_program.effect_token = "mapname".into();
        legacy_mapname.submits[0].ordered_static_map_programs[0]
            .seal_payload_hashes()
            .unwrap();
        let legacy_encoded = serde_json::to_vec(&legacy_mapname).unwrap();
        let legacy_decoded =
            ProductionRenderFrame::decode_json(&legacy_encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap();
        assert_eq!(
            legacy_decoded.submits[0].ordered_static_map_programs[0]
                .mapname_d80_state()
                .unwrap(),
            MapnameD80State::LegacyUnstamped
        );

        let mut authenticated_mapname = legacy_decoded.clone();
        let StaticMapCommand::Draw { draw } = &mut authenticated_mapname.submits[0]
            .ordered_static_map_programs[0]
            .ordered_operations[1]
            .command
        else {
            unreachable!()
        };
        draw.ordered_constant_buffers = vec![mapname_constant_buffer()];
        authenticated_mapname.submits[0].ordered_static_map_programs[0]
            .seal_payload_hashes()
            .unwrap();
        let authenticated_encoded = serde_json::to_vec(&authenticated_mapname).unwrap();
        let authenticated_decoded = ProductionRenderFrame::decode_json(
            &authenticated_encoded,
            TEST_EFFECTIVE_VFS_MANIFEST_SHA256,
        )
        .unwrap();
        assert_eq!(
            authenticated_decoded.submits[0].ordered_static_map_programs[0]
                .mapname_d80_state()
                .unwrap(),
            MapnameD80State::Authenticated
        );

        let mut forged_non_mapname = decoded.clone();
        let StaticMapCommand::Draw { draw } =
            &mut forged_non_mapname.submits[0].ordered_static_map_programs[0].ordered_operations[1]
                .command
        else {
            unreachable!()
        };
        draw.ordered_constant_buffers = vec![mapname_constant_buffer()];
        forged_non_mapname.submits[0].ordered_static_map_programs[0]
            .seal_payload_hashes()
            .unwrap();
        let forged_encoded = serde_json::to_vec(&forged_non_mapname).unwrap();
        assert!(ProductionRenderFrame::decode_json(&forged_encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("semantic is bound to a non-mapname static draw"));

        let mut incomplete_stage = authenticated_decoded.clone();
        let StaticMapCommand::Draw { draw } =
            &mut incomplete_stage.submits[0].ordered_static_map_programs[0].ordered_operations[1]
                .command
        else {
            unreachable!()
        };
        draw.ordered_constant_buffers[0].stage_mask_u8 = 0b01;
        incomplete_stage.submits[0].ordered_static_map_programs[0]
            .seal_payload_hashes()
            .unwrap();
        let incomplete_encoded = serde_json::to_vec(&incomplete_stage).unwrap();
        assert!(ProductionRenderFrame::decode_json(
            &incomplete_encoded,
            TEST_EFFECTIVE_VFS_MANIFEST_SHA256
        )
        .unwrap_err()
            .contains("lacks complete VS+PS coverage"));

        assert_eq!(
            decoded.submits[0].entities[0]
                .content_identity
                .native_selection
                .pass_selector_i32,
            -5
        );
        assert!(decoded.submits[0].ordered_static_map_programs[0]
            .pass_token
            .starts_with("opaque-pass:"));
        assert_eq!(
            decoded.submits[0].particles[0]
                .geometry
                .stream_output_targets[0]
                .byte_offset,
            u32::MAX
        );
        assert!(decoded
            .validate("not-a-digest")
            .unwrap_err()
            .contains("SHA-256 is not 64 lowercase hexadecimal"));
        let mut identity_tampered = decoded.clone();
        identity_tampered.submits[0].ordered_static_map_programs[0].pass_token =
            "opaque-pass:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
        assert!(identity_tampered
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("program payload SHA-256 mismatch"));
        let mut selector_tampered = decoded.clone();
        selector_tampered.submits[0].entities[0]
            .content_identity
            .native_selection
            .mesh_draw_list_index += 1;
        assert!(selector_tampered
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("selector-native identity SHA-256 mismatch"));
        let mut asset_substituted = decoded.clone();
        asset_substituted.submits[0].entities[0].resolved_entity_sha256 = "1".repeat(64);
        assert!(asset_substituted
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("does not match admitted asset payload hash"));
        let mut flags_substituted = decoded.clone();
        flags_substituted.submits[0].entities[0].state_flags ^= 1 << 16;
        assert!(flags_substituted
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("do not equal the selector-native packed tuple"));
        let mut particle_identity_tampered = decoded.clone();
        particle_identity_tampered.submits[0].particles[0]
            .content_identity
            .subsystem_selector_u32 += 1;
        assert!(particle_identity_tampered
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("particle content identity SHA-256 mismatch"));
        let mut texture_substituted = decoded.clone();
        texture_substituted.submits[0].particles[0]
            .content_identity
            .texture_content = content("gfx/textures/substitute.dds");
        texture_substituted.submits[0].particles[0].content_identity_sha256 =
            texture_substituted.submits[0].particles[0]
                .content_identity
                .computed_sha256()
                .unwrap();
        assert!(texture_substituted
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("selected texture winner is absent"));
        let mut half_pixel_substituted = decoded.clone();
        half_pixel_substituted.submits[0].particles[0]
            .content_identity
            .atlas_half_pixel_width_f32_bits ^= 1;
        half_pixel_substituted.submits[0].particles[0].content_identity_sha256 =
            half_pixel_substituted.submits[0].particles[0]
                .content_identity
                .computed_sha256()
                .unwrap();
        assert!(half_pixel_substituted
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("half-pixel bits do not match"));
        // kills: carrier.render_submit.submit_index — the submit no longer carries its
        // identity; it is derived from selected_present.selected_candidate_id (11).
        // kills: carrier.render_submit.presentation_qpc_ticks — presentation ticks are
        // derived from selected_present.selected_qpc_ticks (500), not carried on the submit.
        let encoded_text = String::from_utf8(encoded).unwrap();
        assert!(!encoded_text.contains("\"submit_index\""));
        assert!(!encoded_text.contains("\"presentation_qpc_ticks\""));
        assert_eq!(decoded.retained_submit_identity(), Some(11));
        assert_eq!(decoded.selected_presentation_qpc_ticks(), Some(500));
        let mut tampered = decoded;
        tampered.submits[0].ordered_static_map_programs[0].ordered_operations[0].payload_sha256 =
            "0".repeat(64);
        tampered.submits[0].ordered_static_map_programs[0].payload_sha256 = tampered.submits[0]
            .ordered_static_map_programs[0]
            .computed_payload_sha256()
            .unwrap();
        assert!(tampered
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .unwrap_err()
            .contains("operation payload SHA-256 mismatch"));
    }

    #[test]
    fn v2_carrier_fails_with_explicit_no_exact_conversion_error() {
        let error =
            ProductionRenderFrame::decode_json(br#"{"schema":"hoi4.production-render-submit.v2"}"#, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err();
        assert!(error.contains("incompatible"));
        assert!(error.contains("exact conversion is unavailable"));
        assert!(error.contains("selector-native entity and particle authority"));
    }

    #[test]
    fn v3_carrier_fails_instead_of_reinterpreting_ordinal_as_f32_bits() {
        let error =
            ProductionRenderFrame::decode_json(br#"{"schema":"hoi4.production-render-submit.v3"}"#, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err();
        assert!(error.contains("incompatible"));
        assert!(error.contains("mislabelled as interpolation f32 bits"));
        assert!(error.contains("camera carrier"));
    }

    #[test]
    fn authenticated_camera_carrier_checks_identity_layout_join_and_digest() {
        let current_eye = [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()];
        let current_look = [4.0f32.to_bits(), 5.0f32.to_bits(), 6.0f32.to_bits()];
        let mut bytes = vec![0u8; A729_CAMERA_RENDER_VIEW_BYTES];
        for (index, value) in current_eye.into_iter().enumerate() {
            bytes[0x180 + index * 4..0x184 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        for (index, value) in current_look.into_iter().enumerate() {
            bytes[0x18c + index * 4..0x190 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut raw_event = [0u8; 16];
        raw_event[4..8].copy_from_slice(&4i32.to_le_bytes());
        raw_event[8..12].copy_from_slice(&120i32.to_le_bytes());
        let mut camera = AuthenticatedCameraCarrier {
            schema: A729_CAMERA_TEMPORAL_SCHEMA.into(),
            executable_sha256: A729_CAMERA_EXECUTABLE_SHA256.into(),
            capture_boundary_rva: A729_CAMERA_ENVELOPE_RVA,
            camera_update_rva: A729_CAMERA_UPDATE_RVA,
            carrier_ordinal: 0,
            carrier_offset: 0x10,
            owner_kind: A729CameraOwnerKind::InGame,
            owner_vtable_rva: 0x0295_2510,
            render_view_436_bytes: bytes.clone(),
            canonical_camera_root_436_bytes: bytes,
            previous_current_eye_f32_bits: [0; 3],
            current_eye_f32_bits: current_eye,
            current_look_at_f32_bits: current_look,
            target_eye_f32_bits: [7.0f32.to_bits(); 3],
            target_look_at_f32_bits: [8.0f32.to_bits(); 3],
            viewport_i32: [0, 0, 1920, 1080],
            min_height_i32: 50,
            max_height_i32: 3000,
            update_produced_u8: 1,
            special_free_state_u8: 0,
            ramp_f32_bits: 1.0f32.to_bits(),
            wrap_branch_u8: 1,
            camera_event_batches: vec![A729CameraEventBatch {
                producer_rva: A729_CAMERA_UPDATE_RVA,
                owner_kind: A729CameraOwnerKind::InGame,
                owner_vtable_rva: 0x0295_2510,
                action_source_owner_offset: 0x508,
                cursor_source_owner_offset: 0x500,
                event_list_owner_offset: 0x758,
                frame_scalar_f32_bits: Some(0.1f32.to_bits()),
                producer_flag_0_u8: 1,
                producer_flag_1_u8: 0,
                input_events: vec![A729CameraInputEvent {
                    node_ordinal: 0,
                    node_kind_i32: 0,
                    consumed_before_u8: 0,
                    consumed_after_u8: 1,
                    event_type_i32: 4,
                    signed_delta_i32: 120,
                    embedded_event_16_bytes: raw_event,
                }],
                map_picks: vec![A729CameraMapPick {
                    call_ordinal: 0,
                    mode_u8: 1,
                    screen_xy_f32_bits: [960.0f32.to_bits(), 540.0f32.to_bits()],
                    world_xyz_f32_bits: [10.0f32.to_bits(), 0, 20.0f32.to_bits()],
                }],
                current_eye_before_f32_bits: current_eye,
                target_eye_before_f32_bits: [7.0f32.to_bits(); 3],
                target_look_at_before_f32_bits: [8.0f32.to_bits(); 3],
                current_eye_after_f32_bits: current_eye,
                target_eye_after_f32_bits: [7.0f32.to_bits(); 3],
                target_look_at_after_f32_bits: [8.0f32.to_bits(); 3],
            }],
            payload_sha256: String::new(),
        };
        camera.payload_sha256 = camera.computed_payload_sha256();
        camera.validate().unwrap();

        camera.ramp_f32_bits ^= 1;
        assert!(camera.validate().unwrap_err().contains("payload SHA-256"));

        camera.ramp_f32_bits ^= 1;
        camera.camera_event_batches[0].target_eye_after_f32_bits[0] ^= 1;
        camera.payload_sha256 = camera.computed_payload_sha256();
        assert!(camera
            .validate()
            .unwrap_err()
            .contains("producer result differs"));
    }

    #[test]
    fn decode_value_equals_decode_json_and_rejects_wrong_schemas() {
        let frame = production_frame_with_selected_present();
        let encoded = frame.encode_json(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap();

        let bare = serde_json::to_value(&frame).unwrap();
        assert_eq!(
            ProductionRenderFrame::decode_value(&bare, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap(),
            ProductionRenderFrame::decode_json(&encoded, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap()
        );

        let envelope: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert!(envelope.get("wire_schema").is_some());
        let error = ProductionRenderFrame::decode_value(&envelope, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(
            error.contains("unknown field `frame`"),
            "unexpected: {error}"
        );

        let v2 = serde_json::json!({"schema": INCOMPATIBLE_ENTITY_SELECTOR_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v2, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("incompatible"));
        let v3 = serde_json::json!({"schema": INCOMPATIBLE_CAMERA_CARRIER_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v3, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("camera carrier"));
        let v4 = serde_json::json!({"schema": INCOMPATIBLE_SNOW_MUD_CARRIER_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v4, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("SnowMudData"));
        let v5 = serde_json::json!({"schema": INCOMPATIBLE_TREE_RUNTIME_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v5, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("C07 tree visibility/model/LOD"));
        let v6 = serde_json::json!({"schema": INCOMPATIBLE_S1_SHADOW_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v6, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("L02/L03 shadow projection"));
        let v7 = serde_json::json!({"schema": INCOMPATIBLE_CAMERA_EVENT_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v7, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("raw C03 camera-event producer batches"));
        let v8 = serde_json::json!({"schema": INCOMPATIBLE_SNOW_MUD_PRESENT_LINK_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v8, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("selected-Present candidate-set/cutoff receipt"));
        let v9 = serde_json::json!({"schema": INCOMPATIBLE_WEATHER_CARRIER_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v9, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("weather placement/content/visibility"));
        let v10 = serde_json::json!({"schema": INCOMPATIBLE_SNOW_MUD_FADE_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v10, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("reflected SnowMudFade consumers"));
        let v11 = serde_json::json!({"schema": INCOMPATIBLE_RECORDED_INPUT_OBSERVATION_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v11, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("recorded-input variant"));
        let v16 = serde_json::json!({"schema": INCOMPATIBLE_PRODUCTION_FRAME_CENSUS_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v16, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        // Every member the bump retires has to be NAMED by the refusal, or an
        // operator reading it cannot tell which carrier made their recording stale.
        assert!(error.contains("retired per-frame RenderCheckpoint census"), "{error}");
        assert!(error.contains("display_submit_index"), "{error}");
        assert!(error.contains("per-frame RenderAuthority block"), "{error}");
        let v12 = serde_json::json!({"schema": INCOMPATIBLE_AUTHENTICATED_TEXTURE_SNAPSHOT_SCHEMA});
        let error = ProductionRenderFrame::decode_value(&v12, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("engine-generated textures"));

        // kills: dropping the v15 refusal would let retired tiled point-light
        // payload carriers and live texture digests back through decode as if
        // they were current-schema frames.
        let v15_tiled = serde_json::json!({"schema": INCOMPATIBLE_TILED_LIGHT_PAYLOAD_SCHEMA});
        let error =
            ProductionRenderFrame::decode_value(&v15_tiled, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err();
        assert!(error.contains("retired tiled point-light payload carrier"), "{error}");
        // kills: silent re-admission of the retired always-empty
        // entity-presentation / direct-visual producer carriers - a v13 frame
        // must refuse with the NAMED incompatibility (exact message below),
        // never parse and never fall out as a generic unsupported-schema error.
        let v13 = serde_json::json!({"schema": INCOMPATIBLE_PRESENTATION_CARRIER_SCHEMA});
        let error =
            ProductionRenderFrame::decode_value(&v13, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err();
        assert!(error.contains(INCOMPATIBLE_PRESENTATION_CARRIER_SCHEMA), "{error}");
        assert!(error.contains("always-empty entity-presentation"), "{error}");
        assert!(
            error.contains(
                "instead of deriving presentation from the recorded entity/particle state"
            ),
            "{error}"
        );
        // kills: silent re-admission of the retired derived-runtime carriers -
        // a v14 frame must refuse with the NAMED incompatibility (ordered
        // runtime draw list below), never parse and never fall out as a
        // generic unsupported-schema error.
        let v14 = serde_json::json!({"schema": INCOMPATIBLE_DERIVED_RUNTIME_CARRIER_SCHEMA});
        let error =
            ProductionRenderFrame::decode_value(&v14, TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
                .unwrap_err();
        assert!(error.contains("ordered runtime draw list"), "{error}");
        let unknown = serde_json::json!({"schema": "hoi4.production-render-submit.v99"});
        let error = ProductionRenderFrame::decode_value(&unknown, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("unsupported production render schema"));
        let missing = serde_json::json!({"not_schema": true});
        let error = ProductionRenderFrame::decode_value(&missing, TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
        assert!(error.contains("no string schema"));
    }

    #[test]
    fn particle_identity_order_and_duplicates_are_preserved_and_hashed() {
        let first = particle_content_identity(4);
        let second = particle_content_identity(9);
        let ordered = vec![first.clone(), first.clone(), second.clone()];
        let encoded = serde_json::to_vec(&ordered).unwrap();
        let decoded: Vec<ParticleContentIdentity> = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, ordered);
        assert_eq!(decoded[0], decoded[1]);
        let reordered = vec![second, first.clone(), first];
        assert_ne!(
            canonical_json_sha256(&ordered).unwrap(),
            canonical_json_sha256(&reordered).unwrap()
        );
    }

    #[test]
    fn semantic_entity_selector_labels_are_rejected_by_v3() {
        let mut value = serde_json::to_value(entity_content_identity()).unwrap();
        value.as_object_mut().unwrap().insert(
            "material_variant_token".into(),
            serde_json::json!("invented"),
        );
        let error = serde_json::from_value::<EntityContentIdentity>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn semantic_render_target_aliases_are_rejected_by_v3() {
        for (legacy, alias) in [
            (r#"{"kind":"main_scene_color"}"#, "main_scene_color"),
            (r#"{"kind":"main_scene_depth"}"#, "main_scene_depth"),
            (
                r#"{"kind":"post_process_color","producer_pass_token":"x","attachment_ordinal":0}"#,
                "post_process_color",
            ),
            (
                r#"{"kind":"named_render_graph_resource","resource_token":"x","producer_pass_token":"y","attachment_ordinal":0}"#,
                "named_render_graph_resource",
            ),
        ] {
            let error = serde_json::from_str::<PreparedRenderTarget>(legacy)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("unknown variant `{alias}`")),
                "{legacy} was not rejected as an unknown variant: {error}"
            );
        }
        assert_eq!(
            serde_json::from_str::<PreparedRenderTarget>(
                r#"{"kind":"opaque_render_target","attachment_ordinal":7}"#,
            )
            .unwrap(),
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 7
            }
        );
    }

    #[test]
    fn rejects_semantic_or_truncated_substitutes() {
        let mut truncated_packet = canonical_particle_packet([0; 16], 1, 1, 0, 0, 0);
        truncated_packet.pop();
        let frame = ProductionRenderFrame {
            schema: SCHEMA.into(),
            update_index: 16_959_360,
            first_sequence: 0,
            last_sequence: 0,
            presentation_qpc_frequency_hz: 10_000_000,
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                1,
                0,
                60_759_359,
                1,
                2,
            ),
            snow_mud_texture: None,
            submits: vec![RenderSubmit {
                global_order: 1,
                camera_carrier: None,
                ordered_static_map_programs: vec![],
                authenticated_texture_snapshots: vec![],
                s1_shadow: None,
                particle_packet_set_sha256: "5".repeat(64),
                particles: vec![ParticlePacket {
                    effect_instance_id: 1,
                    registration_token: "x".into(),
                    resolved_definition_sha256: "c".repeat(64),
                    content_identity: particle_content_identity(0),
                    content_identity_sha256: particle_content_identity(0)
                        .computed_sha256()
                        .unwrap(),
                    bucket: 0,
                    packet_ordinal: 0,
                    draw_order: 0,
                    ribbon_tail_order: None,
                    canonical_packet_112_bytes: truncated_packet.clone(),
                    canonical_packet_sha256: hash(&truncated_packet),
                    packet_matrix_4x4_f32_bits: [0; 16],
                    live_count: 1,
                    record_stride: 48,
                    max_count: 1,
                    ordered_gpu_instance_records: vec![0; 48],
                    gpu_instance_records_sha256: hash(&vec![0; 48]),
                    elapsed_f32_bits: 0,
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: particle_geometry(1),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    atlas_rows: 1,
                    atlas_columns: 1,
                    scale_f32_bits: 0,
                    flags_u16: 0,
                }],
                entities: vec![],
                rail_construction_arrows: vec![],
                ordered_render_operations: Vec::new(),
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m03_arrows: None,
            rail_supply_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            authenticated_ui_pass: None,
        };
        assert!(frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err().contains("112"));
    }

    #[test]
    fn exact_render_target_extension_covers_persistent_nonzero_mip_and_mrt() {
        fn exact_target(attachment_ordinal: u32, persistent: bool) -> RenderTargetDesc {
            let mut persistent_key = content(&format!(
                "gfx/modded/runtime_target_{attachment_ordinal}.dds"
            ));
            persistent_key.payload_bytes = 16;
            RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal },
                width: 2,
                height: 2,
                dxgi_format_u32: 28,
                mip_level: 1,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: persistent.then(|| persistent_key.clone()),
                exact_resource: Some(RenderTargetExactResource {
                    resource: RenderTargetResourceDesc {
                        dimension: TextureResourceDimension::Texture2d,
                        width: 4,
                        height: 4,
                        depth: 1,
                        mip_levels: 3,
                        array_size: 1,
                        dxgi_format_u32: 28,
                        sample_count: 1,
                        sample_quality: 0,
                        usage_u32: 0,
                        bind_flags_u32: 0x20,
                        cpu_access_flags_u32: 0,
                        misc_flags_u32: 0,
                    },
                    ordered_views: vec![RenderTargetViewDesc::RenderTarget {
                        dxgi_format_u32: 28,
                        view: RenderTargetAttachmentView::Texture2d { mip_slice: 1 },
                    }],
                    ordered_initial_subresources: persistent
                        .then(|| RenderTargetInitialSubresource {
                            subresource_ordinal: 1,
                            row_pitch: 8,
                            slice_pitch: 16,
                            source: RenderTargetInitialSource::UsedContentRange {
                                content: persistent_key,
                                source_byte_offset: 0,
                                byte_count: 16,
                                range_sha256: "d".repeat(64),
                            },
                        })
                        .into_iter()
                        .collect(),
                }),
            }
        }

        let output = RenderOutputState {
            ordered_color_targets: vec![exact_target(3, true), exact_target(4, false)],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0.0f32.to_bits(),
                top_left_y_f32_bits: 0.0f32.to_bits(),
                width_f32_bits: 2.0f32.to_bits(),
                height_f32_bits: 2.0f32.to_bits(),
                min_depth_f32_bits: 0.0f32.to_bits(),
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        output.validate().unwrap();

        let mut missing_exact = output.clone();
        missing_exact.ordered_color_targets[0].exact_resource = None;
        assert!(missing_exact
            .validate()
            .unwrap_err()
            .contains("persistent/nonzero-mip"));

        let mut wrong_view = output;
        if let Some(exact) = &mut wrong_view.ordered_color_targets[0].exact_resource {
            exact.ordered_views[0] = RenderTargetViewDesc::RenderTarget {
                dxgi_format_u32: 28,
                view: RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
            };
        }
        assert!(wrong_view
            .validate()
            .unwrap_err()
            .contains("selected render-target format/mip"));
    }

    fn rebind_first_shader_resource(draw: &mut StaticMapDraw, source: StaticShaderResourceSource) {
        draw.ordered_shader_resources[0].source = source;
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
    }

    fn dangling_snapshot_source() -> StaticShaderResourceSource {
        StaticShaderResourceSource::AuthenticatedSnapshot {
            snapshot_sha256: "e".repeat(64),
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 29,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
        }
    }

    fn snow_mud_source() -> StaticShaderResourceSource {
        let texel = vec![0u8; (SNOW_MUD_TEXTURE_WIDTH * SNOW_MUD_TEXTURE_HEIGHT) as usize];
        StaticShaderResourceSource::SnowMudTexture {
            producer_ordinal: 1,
            payload_sha256: hash(&texel),
            resource: TextureResourceDesc {
                dimension: TextureResourceDimension::Texture2d,
                width: SNOW_MUD_TEXTURE_WIDTH,
                height: SNOW_MUD_TEXTURE_HEIGHT,
                depth: 1,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 87,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0x8,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
                ordered_subresources: Vec::new(),
            },
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 87,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
        }
    }

    fn frame_with_side_carrier_source(
        carrier: &str,
        source: StaticShaderResourceSource,
    ) -> ProductionRenderFrame {
        match carrier {
            "M03" => {
                let mut frame = production_frame_with_m03_a48_row();
                let m03 = frame.m03_arrows.as_mut().unwrap();
                rebind_first_shader_resource(&mut m03.draws[0].draw, source);
                m03.used_content = m03
                    .draws
                    .iter()
                    .flat_map(|draw| collect_static_draw_used_content(&draw.draw))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                frame
            }
            "UI-pass HUD" => {
                let mut frame = production_frame_with_exact_ui_pass();
                let command = exact_ui_command_mut(&mut frame, 0);
                let HudCommandExecution::ExactPipeline { draw } = &mut command.execution else {
                    panic!("UI-pass ordinal 0 is the exact-pipeline HUD command");
                };
                rebind_first_shader_resource(&mut draw.exact_draw, source);
                rehash_exact_ui_command_and_pass(&mut frame, 0);
                frame
            }
            other => panic!("unknown side carrier {other}"),
        }
    }

    #[test]
    fn side_carrier_draws_refuse_unresolvable_named_payloads() {
        for carrier in ["M03", "UI-pass HUD"] {
            let frame = frame_with_side_carrier_source(carrier, dangling_snapshot_source());
            assert!(frame
                .submits
                .iter()
                .all(|submit| submit.authenticated_texture_snapshots.is_empty()));
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            assert_eq!(
                error,
                format!(
                    "{carrier} draw references an absent authenticated texture snapshot packet"
                ),
                "{carrier}"
            );

            let frame = frame_with_side_carrier_source(
                carrier,
                StaticShaderResourceSource::PoliticalTexture {
                    texture_kind: PoliticalTextureKind::Secondary,
                    target_binding_identity: "ProvinceSecondaryColorMap".into(),
                    upload_order: 7,
                    payload_sha256: "e".repeat(64),
                    resource: texture().resource,
                    view: texture().view,
                },
            );
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            assert_eq!(
                error,
                format!("{carrier} draw references an absent political texture packet"),
                "{carrier}"
            );

            // No lawful tiled-light payload exists for any carrier until the
            // owner proves the semantic census, overlays included.
            let frame = frame_with_side_carrier_source(
                carrier,
                StaticShaderResourceSource::TiledLightIndex {
                    writer_index: 3,
                    resource: texture().resource,
                    view: texture().view,
                },
            );
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            assert_eq!(
                format!(
                    "{carrier} draw binds a tiled point-light index texture, but no lawful payload exists until the owner proves the semantic point-light census"
                ),
                error,
                "{carrier}"
            );

            let frame = frame_with_side_carrier_source(
                carrier,
                StaticShaderResourceSource::PreparedTarget {
                    target: PreparedRenderTarget::OpaqueRenderTarget {
                        attachment_ordinal: 1,
                    },
                    resource: prepared_target_resource(),
                    view: texture().view,
                    producer: RenderExecutionRef::Static {
                        operation_ordinal: 1,
                    },
                },
            );
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            let expected = if carrier == "UI-pass HUD" {
                format!("{carrier} draw binds a prepared-target SRV with no authenticated producer position in the render-operation order")
            } else {
                "prepared-target SRV does not name its exact preceding last writer".to_string()
            };
            assert_eq!(error, expected, "{carrier}");
        }
    }

    fn prepared_target_resource() -> TextureResourceDesc {
        let mut resource = texture().resource;
        resource.ordered_subresources.clear();
        resource
    }

    #[test]
    fn ordered_side_carrier_draw_admits_its_exact_preceding_last_writer() {
        let mut frame = production_frame_with_m03_a48_row();
        let producer_target = output_state().ordered_color_targets[0].clone();

        let static_texture = texture();
        let mut program = StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: format!("opaque-pass:{}", "a".repeat(64)),
            ordered_content_keys: vec![],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 7,
                payload_sha256: String::new(),
                command: StaticMapCommand::Draw {
                    draw: StaticMapDraw {
                        draw_token: "terrain-producer".into(),
                        shader_program: shader(),
                        ordered_shader_resources: vec![StaticShaderResourceBinding {
                            stage: static_texture.stage,
                            slot: static_texture.slot,
                            semantic_token: static_texture.semantic_token,
                            source: StaticShaderResourceSource::UsedContent {
                                content: static_texture.content,
                                resource: static_texture.resource,
                                view: static_texture.view,
                            },
                        }],
                        pipeline: pipeline(),
                        geometry: StaticGeometrySource::Exact {
                            geometry: entity_geometry(),
                        },
                        ordered_constant_buffers: vec![constant_buffer()],
                        output_state: output_state(),
                        payload_sha256: String::new(),
                    },
                },
            }],
            payload_sha256: String::new(),
        };
        program.seal_payload_hashes().unwrap();
        // kills: carrier.render_submit.ordered_scene_constant_buffers — the submit-level
        // carrier is retired; program frames validate on content alone.
        frame.submits[0].ordered_static_map_programs = vec![program];
        let producer = RenderExecutionRef::Static {
            operation_ordinal: 7,
        };
        frame.submits[0]
            .ordered_render_operations
            .insert(0, producer.clone());

        rebind_first_shader_resource(
            &mut frame.m03_arrows.as_mut().unwrap().draws[0].draw,
            StaticShaderResourceSource::PreparedTarget {
                target: producer_target.prepared_target.clone(),
                resource: prepared_target_resource(),
                view: texture().view,
                producer: producer.clone(),
            },
        );
        let m03 = frame.m03_arrows.as_mut().unwrap();
        m03.used_content = m03
            .draws
            .iter()
            .flat_map(|draw| collect_static_draw_used_content(&draw.draw))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        frame
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .expect("an M03 draw may sample the target its exact preceding static draw wrote");

        let mut wrong = frame;
        rebind_first_shader_resource(
            &mut wrong.m03_arrows.as_mut().unwrap().draws[0].draw,
            StaticShaderResourceSource::PreparedTarget {
                target: producer_target.prepared_target,
                resource: prepared_target_resource(),
                view: texture().view,
                producer: RenderExecutionRef::Static {
                    operation_ordinal: 9,
                },
            },
        );
        assert_eq!(
            wrong.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "prepared-target SRV does not name its exact preceding last writer"
        );
    }

    #[test]
    fn ordered_side_carrier_draw_becomes_the_last_writer_of_its_own_target() {
        let mut frame = production_frame_with_m03_a48_row();
        let producer_target = frame.m03_arrows.as_ref().unwrap().draws[0]
            .draw
            .output_state
            .ordered_color_targets[0]
            .clone();
        let stale_static = RenderExecutionRef::Static {
            operation_ordinal: 7,
        };

        let mut overwriting = frame.m03_arrows.as_ref().unwrap().draws[0].clone();
        overwriting.engine_draw_ordinal = 24;
        let first_ordinal = frame.m03_arrows.as_ref().unwrap().draws[0].engine_draw_ordinal;
        rebind_first_shader_resource(
            &mut overwriting.draw,
            StaticShaderResourceSource::PreparedTarget {
                target: producer_target.prepared_target.clone(),
                resource: prepared_target_resource(),
                view: texture().view,
                producer: stale_static,
            },
        );
        let m03 = frame.m03_arrows.as_mut().unwrap();
        overwriting.members[0].member_ordinal = 0;
        m03.rows[0].draw_members.push(M03ArrowDrawMemberRef {
            engine_draw_ordinal: 24,
            member_ordinal: 0,
        });
        m03.draws.push(overwriting);
        m03.ordered_draws.push(M03ArrowDrawRef {
            engine_draw_ordinal: 24,
        });
        frame.submits[0]
            .ordered_render_operations
            .push(RenderExecutionRef::M03 {
                output: M03ArrowDrawRef {
                    engine_draw_ordinal: 24,
                },
            });

        assert_eq!(
            frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "prepared-target SRV does not name its exact preceding last writer"
        );

        let m03 = frame.m03_arrows.as_mut().unwrap();
        rebind_first_shader_resource(
            &mut m03.draws[1].draw,
            StaticShaderResourceSource::PreparedTarget {
                target: producer_target.prepared_target,
                resource: prepared_target_resource(),
                view: texture().view,
                producer: RenderExecutionRef::M03 {
                    output: M03ArrowDrawRef {
                        engine_draw_ordinal: first_ordinal,
                    },
                },
            },
        );
        frame
            .validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256)
            .expect("an M03 draw may sample the target the preceding M03 draw wrote");
    }

    #[test]
    fn side_carrier_snow_mud_source_requires_the_h_packet() {
        for carrier in ["M03", "UI-pass HUD"] {
            let frame = frame_with_side_carrier_source(carrier, snow_mud_source());
            assert!(frame.snow_mud_texture.is_none());
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            assert_eq!(
                error, "selected-submit SnowMudTexture source has no H packet",
                "{carrier}"
            );
        }
    }

    #[test]
    fn side_carrier_controls_still_pass_and_still_refuse_a_foreign_manifest() {
        for carrier in ["M03", "UI-pass HUD"] {
            let mut frame = match carrier {
                "M03" => production_frame_with_m03_a48_row(),
                _ => production_frame_with_exact_ui_pass(),
            };
            frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_or_else(|error| {
                panic!("{carrier} control frame must remain valid: {error}")
            });

            if carrier == "UI-pass HUD" {
                let command = exact_ui_command_mut(&mut frame, 0);
                let HudCommandExecution::ExactPipeline { draw } = &mut command.execution else {
                    panic!("UI-pass ordinal 0 is the exact-pipeline HUD command");
                };
                let original = draw.exact_draw.shader_program.source.clone();
                draw.exact_draw
                    .shader_program
                    .source
                    .effective_vfs_manifest_sha256 = "9".repeat(64);
                draw.exact_draw.payload_sha256 = draw.exact_draw.computed_payload_sha256().unwrap();
                for content in &mut command.ordered_effective_vfs_used_content {
                    if *content == original {
                        content.effective_vfs_manifest_sha256 = "9".repeat(64);
                    }
                }
                rehash_exact_ui_command_and_pass(&mut frame, 0);
            } else {
                let m03 = frame.m03_arrows.as_mut().unwrap();
                let original = m03.draws[0].draw.shader_program.source.clone();
                m03.draws[0]
                    .draw
                    .shader_program
                    .source
                    .effective_vfs_manifest_sha256 = "9".repeat(64);
                m03.draws[0].draw.payload_sha256 =
                    m03.draws[0].draw.computed_payload_sha256().unwrap();
                for content in &mut m03.used_content {
                    if *content == original {
                        content.effective_vfs_manifest_sha256 = "9".repeat(64);
                    }
                }
                m03.used_content.sort();
            }
            let error = frame.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err();
            assert!(error.contains("content authority"), "{carrier}: {error}");
        }
    }

    #[test]
    fn submitted_draw_content_authority_is_bound_to_the_frame_manifest() {
        let base = production_frame_with_static_entities_and_particles();
        base.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).expect("baseline frame is valid");

        let mut foreign = base.clone();
        let StaticMapCommand::Draw { draw } =
            &mut foreign.submits[0].ordered_static_map_programs[0].ordered_operations[1].command
        else {
            unreachable!("operation 101 is the static draw")
        };
        draw.shader_program.source.effective_vfs_manifest_sha256 = "9".repeat(64);
        foreign.submits[0].ordered_static_map_programs[0]
            .seal_payload_hashes()
            .unwrap();
        assert_eq!(
            foreign.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "static map draw content authority differs from production frame"
        );

        let mut foreign = base.clone();
        foreign.submits[0].entities[0].ordered_texture_bindings[0]
            .content
            .effective_vfs_manifest_sha256 = "9".repeat(64);
        assert_eq!(
            foreign.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "entity content authority differs from production frame"
        );

        let mut foreign = base.clone();
        foreign.submits[0].entities[0]
            .content_identity
            .mesh
            .effective_vfs_manifest_sha256 = "9".repeat(64);
        assert_eq!(
            foreign.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "entity content authority differs from production frame"
        );

        let mut foreign = base.clone();
        foreign.submits[0].particles[0]
            .shader_program
            .source
            .effective_vfs_manifest_sha256 = "9".repeat(64);
        assert_eq!(
            foreign.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "particle content authority differs from production frame"
        );

        let mut foreign = base;
        let particle = &mut foreign.submits[0].particles[0];
        particle.ordered_texture_bindings[0]
            .content
            .effective_vfs_manifest_sha256 = "9".repeat(64);
        particle
            .content_identity
            .texture_content
            .effective_vfs_manifest_sha256 = "9".repeat(64);
        particle.content_identity_sha256 = particle.content_identity.computed_sha256().unwrap();
        assert_eq!(
            foreign.validate(TEST_EFFECTIVE_VFS_MANIFEST_SHA256).unwrap_err(),
            "particle content authority differs from production frame"
        );
    }
}

#[cfg(test)]
mod shared_hour_window_regression {
    #[test]
    fn the_update_index_window_matches_the_shared_admission_window() {
        for probe in [
            portable_admission::RAW_DATE_MIN - 1,
            portable_admission::RAW_DATE_MIN,
            portable_admission::RAW_DATE_MIN + 1,
            portable_admission::RAW_DATE_MAX - 1,
            portable_admission::RAW_DATE_MAX,
            portable_admission::RAW_DATE_MAX + 1,
        ] {
            let shared = portable_admission::hour_index_from_raw(probe)
                .and_then(|hour| u64::try_from(hour).ok());
            assert_eq!(
                super::compact_update_index_from_raw(probe),
                shared,
                "compact_update_index_from_raw disagrees with the shared \
                 admission window at raw date {probe}"
            );
        }
    }

    #[test]
    fn the_upper_endpoint_of_the_window_is_admitted() {
        assert!(
            super::compact_update_index_from_raw(portable_admission::RAW_DATE_MAX).is_some(),
            "RAW_DATE_MAX is inside the window; the superseded `>=` spelling \
             rejected exactly this value while every other copy admitted it"
        );
        assert!(
            super::compact_update_index_from_raw(portable_admission::RAW_DATE_MAX + 1).is_none()
        );
    }
}
pub mod land_combat_strength;
