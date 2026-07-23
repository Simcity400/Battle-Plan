//! Dormant exact-a729 technology-canvas observer and pointer-free carrier.
//!
//! The retained controller roots prove where the mode-6
//! `CCountryTechTreeView` is owned and where population/refresh commits occur,
//! and the exact20 root/helper receipt and sentinels are now frozen. A later
//! literal lookup proved that `controller+0x578` is `close_button`, not a
//! two-dimensional canvas or axis owner. The active folder roots instead live
//! in the populated vector at `controller+0x19f8` (count `+0x1a04`, active
//! ordinal `+0x1a90`). Static evidence does not close their final
//! selected-Present geometry, origin, scale, clipping, or draw order. It also
//! proved that connector segment objects are reused across
//! logical edges, so a post-return pool scan cannot recover portable endpoint
//! identity. This module therefore supplies a complete fail-closed engineering
//! seam while
//! [`PRODUCTION_ACTIVATION`] remains false. The carrier is wired end-to-end on
//! reserved domain 71, but collection and authenticated decode remain disabled
//! unless the schema is revised for the single 2D root and a contextual
//! connector producer is implemented.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

pub const MAGIC: u32 = u32::from_le_bytes(*b"TCV1");
pub const SCHEMA_VERSION: u32 = 2;
pub const PRODUCTION_ACTIVATION: bool = false;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

/// Final exact-a729 static-exhaustion exports. These close the controller-root
/// and connector-pool questions; they do not manufacture the absent TCV1 axis
/// records or portable connector identities.
pub const FINAL_CONTROLLER_CONNECTOR_JOB_ID: &str =
    "20260722T072211Z-a28-tech-canvas-connector-two-axis-final-1b34e6a9";
pub const FINAL_CONTROLLER_CONNECTOR_MANIFEST_SHA256: &str =
    "c11f54bc04a2ea34fb50d967bbfd87b3acfbe069774c2cd24bc931d778360c88";
pub const FINAL_OBJECT_SCROLL_JOB_ID: &str =
    "20260722T072341Z-a28-tech-canvas-object-scroll-final-fd03b189";
pub const FINAL_OBJECT_SCROLL_MANIFEST_SHA256: &str =
    "49a33352fa2e5dbf577c1f2a87562aaa646a61ff48bd8c6d20afda64e9554de1";
pub const FINAL_PAN_OWNER_JOB_ID: &str =
    "20260722T072548Z-a28-tech-canvas-pan-scroll-owner-final-85776e34";
pub const FINAL_PAN_OWNER_MANIFEST_SHA256: &str =
    "08b91afcf7b313a5be968990f6d8e94d041386aec1b4557e4c567451a2a01864";

pub const OWNER_CONSTRUCTOR_RVA: usize = 0x00B5_2390;
pub const COUNTRY_TECH_TREE_CONSTRUCTOR_RVA: usize = 0x013B_E7F0;
pub const POPULATION_COMMIT_RVA: usize = 0x013C_7410;
pub const FOLDER_MATERIALIZER_RVA: usize = 0x013C_1390;
pub const REFRESH_COMMIT_RVA: usize = 0x013C_4890;
pub const ITEM_REBIND_RVA: usize = 0x013C_8B70;
pub const CALLBACK_OBSERVER_GLUE_RVA: usize = 0x013B_E010;
pub const GRID_LOOKUP_MAP_INSERT_RVA: usize = 0x013B_D410;
pub const TECH_ITEM_POOL_ACQUIRE_RVA: usize = 0x013B_DD80;
pub const POST_POPULATION_TAB_BIND_RVA: usize = 0x013B_F850;
pub const CONNECTOR_PATH_MATERIALIZER_RVA: usize = 0x013C_10A0;
pub const GRID_RESOLVER_RVA: usize = 0x013C_3040;
pub const MATERIALIZED_VECTOR_CLEAR_RVA: usize = 0x013C_4480;
/// Internal callback code labels installed by the typed constructor. A
/// function-root export rejected this address class; the direct bytes are
/// retained separately and these values are not exact20 function sites.
pub const INTERNAL_OBSERVER_CALLBACK_LABEL_RVAS: [usize; 3] =
    [0x013C_4550, 0x013C_45B0, 0x013C_4720];
pub const REQUIRED_EXACT20_FUNCTION_RVAS: [usize; 13] = [
    OWNER_CONSTRUCTOR_RVA,
    COUNTRY_TECH_TREE_CONSTRUCTOR_RVA,
    POPULATION_COMMIT_RVA,
    FOLDER_MATERIALIZER_RVA,
    REFRESH_COMMIT_RVA,
    ITEM_REBIND_RVA,
    CALLBACK_OBSERVER_GLUE_RVA,
    GRID_LOOKUP_MAP_INSERT_RVA,
    TECH_ITEM_POOL_ACQUIRE_RVA,
    POST_POPULATION_TAB_BIND_RVA,
    CONNECTOR_PATH_MATERIALIZER_RVA,
    GRID_RESOLVER_RVA,
    MATERIALIZED_VECTOR_CLEAR_RVA,
];
pub const MODE6_OWNER_SLOT_OFFSET: usize = 0xF8;
pub const MODE7_OWNER_SLOT_OFFSET: usize = 0x100;
pub const CONTROLLER_MODE_OFFSET: usize = 0x1AA4;
pub const TECHNOLOGY_MODE: u32 = 6;

const MAX_STR: usize = 16_384;
const MAX_DRAWABLES: usize = 131_072;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSiteContract {
    pub rva: usize,
    pub entry_sentinel: [u8; 16],
}

/// Frozen only from the same-session exact20 receipt covering every listed
/// root/helper. Placeholder bytes or retained exact10 manifests are not valid
/// substitutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exact20HookContract {
    pub exact20_receipt_sha256: &'static str,
    pub root_producer_manifest_sha256: &'static str,
    pub helper_producer_manifest_sha256: &'static str,
    /// Ordered exactly as [`REQUIRED_EXACT20_FUNCTION_RVAS`]. Each sentinel is
    /// receipt-derived; non-hook producer roots are included because they
    /// define identity, filtering, order, geometry, or scroll ownership.
    pub function_sites: [FunctionSiteContract; 13],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveHookInstallationContract {
    pub callback_rva: usize,
    pub installed_patch_sha256: &'static str,
    pub trampoline_sha256: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLiveProducerContract {
    pub live_observation_receipt_sha256: &'static str,
    pub live_producer_manifest_sha256: &'static str,
    pub owner_registry_manifest_sha256: &'static str,
    pub selected_present_producer_manifest_sha256: &'static str,
    pub owner_hook: LiveHookInstallationContract,
    pub population_hook: LiveHookInstallationContract,
    pub refresh_hook: LiveHookInstallationContract,
}

pub const AUTHENTICATED_EXACT20_HOOK_CONTRACT: Option<Exact20HookContract> =
    Some(Exact20HookContract {
        exact20_receipt_sha256:
            "6545f79563654c874cb078b3274d0f727237c093e6e845211f3f9672ee265f48",
        root_producer_manifest_sha256:
            "eab85630d79f7592bd1ae38f9ae5fb6102ddb5e2201cb3a1d6482a83cf5c74cb",
        helper_producer_manifest_sha256:
            "9837e5da6e0fb0c9cc4efdc49b1351b71a69810e5f94968e726da75679b2df5e",
        function_sites: [
            FunctionSiteContract {
                rva: OWNER_CONSTRUCTOR_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x20\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41",
            },
            FunctionSiteContract {
                rva: COUNTRY_TECH_TREE_CONSTRUCTOR_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x10\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41",
            },
            FunctionSiteContract {
                rva: POPULATION_COMMIT_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x08\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57",
            },
            FunctionSiteContract {
                rva: FOLDER_MATERIALIZER_RVA,
                entry_sentinel: *b"\x48\x8b\xc4\x4c\x89\x48\x20\x48\x89\x50\x10\x48\x89\x48\x08\x55",
            },
            FunctionSiteContract {
                rva: REFRESH_COMMIT_RVA,
                entry_sentinel: *b"\x48\x8b\xc4\x48\x89\x40\x08\x55\x53\x56\x57\x41\x54\x41\x55\x41",
            },
            FunctionSiteContract {
                rva: ITEM_REBIND_RVA,
                entry_sentinel: *b"\x40\x53\x57\x41\x54\x41\x55\x48\x83\xec\x58\x4c\x8b\xe9\xe8\xed",
            },
            FunctionSiteContract {
                rva: CALLBACK_OBSERVER_GLUE_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x10\x48\x89\x74\x24\x18\x48\x89\x7c\x24\x20\x48",
            },
            FunctionSiteContract {
                rva: GRID_LOOKUP_MAP_INSERT_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x18\x55\x56\x57\x41\x56\x41\x57\x48\x8b\xec\x48",
            },
            FunctionSiteContract {
                rva: TECH_ITEM_POOL_ACQUIRE_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x10\x48\x89\x6c\x24\x18\x48\x89\x74\x24\x20\x57",
            },
            FunctionSiteContract {
                rva: POST_POPULATION_TAB_BIND_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x08\x48\x89\x74\x24\x18\x48\x89\x7c\x24\x20\x48",
            },
            FunctionSiteContract {
                rva: CONNECTOR_PATH_MATERIALIZER_RVA,
                entry_sentinel: *b"\x48\x89\x54\x24\x10\x53\x55\x56\x57\x41\x56\x48\x83\xec\x60\x4c",
            },
            FunctionSiteContract {
                rva: GRID_RESOLVER_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x18\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41",
            },
            FunctionSiteContract {
                rva: MATERIALIZED_VECTOR_CLEAR_RVA,
                entry_sentinel: *b"\x48\x89\x5c\x24\x18\x56\x48\x83\xec\x20\x8b\x81\x4c\x1a\x00\x00",
            },
        ],
    });

/// The jobs performed in-broker readiness checks, but no same-session status
/// artifact was persisted before their leases. Keep the dependent production
/// gate absent until the canonical pool/job ledger supplies one.
pub const PERSISTED_EXACT20_PREFLIGHT_RECEIPT_SHA256: Option<&str> = None;

/// Deliberately absent. The retained static exports identify the active folder
/// root but do not bind it to an authenticated selected Present or recover its
/// exact final screen-space origin/scale/clip/order. Portable connector identity
/// additionally requires a contextual bracket around
/// `CONNECTOR_PATH_MATERIALIZER_RVA` and every segment-helper emission because
/// the post-return pool merges/reuses segment objects. None of these values may
/// be synthesized from the final pool.
pub const AUTHENTICATED_LIVE_PRODUCER_CONTRACT: Option<AuthenticatedLiveProducerContract> = None;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ObserverBoundary {
    OwnerConstructed = 1,
    PopulationCommitted = 2,
    RefreshCommitted = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ObservationPhase {
    /// The authenticated function was entered, but its mutations are not yet
    /// complete. This phase is identity evidence only and cannot emit.
    FunctionEntry = 1,
    /// The detour wrapper has returned from the authenticated population or
    /// refresh function, so its mutations are complete for this transition.
    PostReturn = 2,
}

impl ObserverBoundary {
    fn from_wire(value: u32) -> Result<Self, ObserverError> {
        match value {
            1 => Ok(Self::OwnerConstructed),
            2 => Ok(Self::PopulationCommitted),
            3 => Ok(Self::RefreshCommitted),
            _ => invalid("unknown observer boundary"),
        }
    }

    fn is_complete_snapshot_boundary(self) -> bool {
        matches!(self, Self::PopulationCommitted | Self::RefreshCommitted)
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DrawableKind {
    TechnologyNode = 1,
    ConnectorSegment = 2,
    ConnectorJoint = 3,
    ControllerOverlay = 4,
}

impl DrawableKind {
    fn from_wire(value: u32) -> Result<Self, ObserverError> {
        match value {
            1 => Ok(Self::TechnologyNode),
            2 => Ok(Self::ConnectorSegment),
            3 => Ok(Self::ConnectorJoint),
            4 => Ok(Self::ControllerOverlay),
            _ => invalid("unknown technology drawable kind"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisStateFixed {
    pub min: i64,
    pub max: i64,
    pub current: i64,
    pub target: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyCanvasAuthority {
    pub executable_sha256: String,
    pub exact20_receipt_sha256: String,
    pub exact20_preflight_receipt_sha256: String,
    pub root_producer_manifest_sha256: String,
    pub helper_producer_manifest_sha256: String,
    pub live_observation_receipt_sha256: String,
    pub live_producer_manifest_sha256: String,
    pub owner_registry_manifest_sha256: String,
    /// Authenticated receipt for the exact selected Present whose final UI
    /// state is carried by this frame. A same-game-hour receipt is not enough.
    pub selected_present_receipt_sha256: String,
    pub selected_present_pass_identity_sha256: String,
    pub selected_present_producer_manifest_sha256: String,
    pub content_manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerRegistryWitness {
    pub registry_epoch: u64,
    pub owner_sequence: u64,
    pub construction_qpc_ticks: u64,
    pub owner_construction_record_sha256: String,
    pub registry_manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologySourceItem {
    pub stable_item_id: String,
    pub hierarchy_path: String,
    pub folder_id: String,
    pub source_ordinal: u32,
    pub kind: DrawableKind,
    pub included: bool,
    pub exclusion_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedDrawable {
    pub drawable_id: String,
    pub hierarchy_path: String,
    pub slot_id: String,
    pub draw_ordinal: u32,
    pub rect: RectI32,
    /// Exact selected-Present UI operation ordinals emitted for this
    /// drawable by the same GUI traversal. Container-rectangle matching is
    /// explicitly not a valid substitute.
    pub ui_pass_ordinals: Vec<u32>,
}

/// Complete, portable callback-to-selected-Present technology-canvas
/// observation. No raw process pointer is representable in this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyCanvasFrame {
    pub authority: TechnologyCanvasAuthority,
    pub source_record_id: String,
    pub owner_instance_id: String,
    pub frame_hour: u64,
    pub elapsed_wall_ms: u64,
    pub capture_sequence: u64,
    /// QPC at the completed population/refresh callback.
    pub callback_qpc_ticks: u64,
    /// Monotonic identity and QPC of the selected Present that consumed this
    /// active technology root. These bind callback state to one render submit;
    /// game hour alone is deliberately insufficient.
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_cutoff_qpc_ticks: u64,
    /// Pointer-free token assigned by the selected-Present producer to the
    /// exact runtime GUI instance registered by the technology hook.
    pub selected_present_engine_instance_token: u64,
    pub boundary: ObserverBoundary,
    pub controller_mode: u32,
    pub authored_surface_identity: String,
    pub runtime_root_identity: String,
    pub effective_interface_winner: String,
    pub active_root_identity: String,
    pub source_items: Vec<TechnologySourceItem>,
    pub final_draw_order: Vec<String>,
    /// Strictly ordered complete union of all drawable `ui_pass_ordinals`.
    pub selected_present_ui_operation_order: Vec<u32>,
    pub materialized_drawables: Vec<MaterializedDrawable>,
    pub clip_rect: RectI32,
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    pub render_target_extent_pixels: [i32; 2],
    pub client_extent_pixels: [i32; 2],
    /// Exact fixed-5 screen-space origin of the selected active folder root.
    pub canvas_origin_pixels_fixed5: [i64; 2],
    /// Raw IEEE-754 bit patterns for the exact selected-Present X/Y canvas
    /// scale. The producer must read or statically prove these values; identity
    /// scale must never be assumed.
    pub canvas_scale_bits: [u32; 2],
    /// Explicit fixed-5 pixel translation. It is not reconstructed from either
    /// axis' scroll current value.
    pub pan_pixels_fixed5: [i64; 2],
    pub horizontal_scroll_fixed5: AxisStateFixed,
    pub vertical_scroll_fixed5: AxisStateFixed,
}

/// Process-local callback observation. These pointer fields are accepted only
/// by an authenticated bridge and never enter [`TechnologyCanvasFrame`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawBoundaryObservation {
    pub callback_rva: usize,
    /// Original bytes saved and authenticated before an inline detour mutates
    /// the entry. Current patched bytes are bound separately by hash.
    pub original_entry_bytes_before_patch: [u8; 16],
    pub installed_patch_sha256: String,
    pub trampoline_sha256: String,
    pub phase: ObservationPhase,
    pub owner_root_ptr: usize,
    pub controller_ptr: usize,
    pub owner_mode6_slot_value: usize,
    pub owner_registry_witness: OwnerRegistryWitness,
    pub controller_mode_raw: u32,
    pub frame_hour: u64,
    pub elapsed_wall_ms: u64,
    pub capture_sequence: u64,
    pub callback_qpc_ticks: u64,
    pub boundary: ObserverBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverError {
    Inactive(&'static str),
    Unauthenticated(String),
    Invalid(String),
    Source(String),
    Truncated,
    TrailingBytes,
}

pub(crate) mod private {
    pub trait Sealed {}
}

/// Implemented only by the eventual exact-a729 hook/registry bridge inside
/// this crate. It must erase pointers and return complete producer accounting.
pub trait ExactTechnologyCanvasObserverSource: private::Sealed {
    fn authority(&self) -> TechnologyCanvasAuthority;
    fn observe_complete_transition(
        &self,
        raw: &RawBoundaryObservation,
    ) -> Result<TechnologyCanvasFrame, String>;
}

pub fn activation_blockers() -> Vec<&'static str> {
    let mut blockers = Vec::new();
    if !PRODUCTION_ACTIVATION {
        blockers.push("compile-time H-A29 technology-canvas activation is false");
    }
    if AUTHENTICATED_EXACT20_HOOK_CONTRACT.is_none() {
        blockers.push("authenticated exact20 hook receipt/sentinels are absent");
    }
    if PERSISTED_EXACT20_PREFLIGHT_RECEIPT_SHA256.is_none() {
        blockers.push("persisted same-session exact20 preflight receipt is absent");
    }
    if AUTHENTICATED_LIVE_PRODUCER_CONTRACT.is_none() {
        blockers.push("no authenticated selected-Present producer resolves controller+0x19f8[controller+0x1a90] to a portable active-root identity and exact final origin/scale/clip/extents/order; controller+0x578 is close_button, and reused connector objects cannot yield portable endpoint identity from a post-return pool scan");
    }
    blockers
}

pub fn expected_source_record_id(
    owner_instance_id: &str,
    frame_hour: u64,
    capture_sequence: u64,
    callback_qpc_ticks: u64,
    selected_present_candidate_id: u64,
    selected_present_ordinal: u32,
    selected_present_qpc_ticks: u64,
    selected_present_cutoff_qpc_ticks: u64,
    selected_present_engine_instance_token: u64,
    boundary: ObserverBoundary,
    authored_surface_identity: &str,
    runtime_root_identity: &str,
    effective_interface_winner: &str,
    active_root_identity: &str,
    selected_present_receipt_sha256: &str,
    selected_present_pass_identity_sha256: &str,
    selected_present_producer_manifest_sha256: &str,
    exact20_preflight_receipt_sha256: &str,
    root_producer_manifest_sha256: &str,
    helper_producer_manifest_sha256: &str,
    live_observation_receipt_sha256: &str,
    live_producer_manifest_sha256: &str,
    owner_registry_manifest_sha256: &str,
    content_manifest_sha256: &str,
) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"the game:a729:technology-canvas-source-record:v2\0");
    put_len_prefixed_unchecked(&mut canonical, owner_instance_id.as_bytes());
    put_u64(&mut canonical, frame_hour);
    put_u64(&mut canonical, capture_sequence);
    put_u64(&mut canonical, callback_qpc_ticks);
    put_u64(&mut canonical, selected_present_candidate_id);
    put_u32(&mut canonical, selected_present_ordinal);
    put_u64(&mut canonical, selected_present_qpc_ticks);
    put_u64(&mut canonical, selected_present_cutoff_qpc_ticks);
    put_u64(&mut canonical, selected_present_engine_instance_token);
    put_u32(&mut canonical, boundary as u32);
    put_len_prefixed_unchecked(&mut canonical, authored_surface_identity.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, runtime_root_identity.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, effective_interface_winner.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, active_root_identity.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, selected_present_receipt_sha256.as_bytes());
    put_len_prefixed_unchecked(
        &mut canonical,
        selected_present_pass_identity_sha256.as_bytes(),
    );
    put_len_prefixed_unchecked(
        &mut canonical,
        selected_present_producer_manifest_sha256.as_bytes(),
    );
    put_len_prefixed_unchecked(&mut canonical, exact20_preflight_receipt_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, root_producer_manifest_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, helper_producer_manifest_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, live_observation_receipt_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, live_producer_manifest_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, owner_registry_manifest_sha256.as_bytes());
    put_len_prefixed_unchecked(&mut canonical, content_manifest_sha256.as_bytes());
    let digest = Sha256::digest(canonical);
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    format!("a729:technology_canvas:{}", hex32(array))
}

/// Future hook entrypoint. Even a structurally complete fake source cannot
/// emit while activation or either live evidence contract is absent.
pub fn collect_after_boundary<S: ExactTechnologyCanvasObserverSource>(
    source: &S,
    raw: RawBoundaryObservation,
) -> Result<Vec<u8>, ObserverError> {
    if !PRODUCTION_ACTIVATION {
        return Err(ObserverError::Inactive(
            "H-A29 technology canvas observer is compile-time inactive",
        ));
    }
    let live_contract = AUTHENTICATED_LIVE_PRODUCER_CONTRACT.ok_or(
        ObserverError::Inactive("H-A29 authenticated live producer contract is absent"),
    )?;
    let persisted_preflight = PERSISTED_EXACT20_PREFLIGHT_RECEIPT_SHA256.ok_or(
        ObserverError::Inactive("H-A29 persisted exact20 preflight receipt is absent"),
    )?;
    if !lower_hex_sha256(persisted_preflight) {
        return Err(ObserverError::Unauthenticated(
            "persisted exact20 preflight receipt is malformed".into(),
        ));
    }
    let contract = AUTHENTICATED_EXACT20_HOOK_CONTRACT.ok_or(ObserverError::Inactive(
        "H-A29 exact20 hook contract is absent",
    ))?;
    validate_exact20_contract(contract)?;
    validate_live_contract(live_contract)?;
    validate_raw_boundary(&raw, contract, live_contract)?;
    let authority = source.authority();
    validate_authority(&authority)?;
    if authority.exact20_receipt_sha256 != contract.exact20_receipt_sha256
        || authority.exact20_preflight_receipt_sha256 != persisted_preflight
        || authority.root_producer_manifest_sha256 != contract.root_producer_manifest_sha256
        || authority.helper_producer_manifest_sha256 != contract.helper_producer_manifest_sha256
        || authority.live_observation_receipt_sha256
            != live_contract.live_observation_receipt_sha256
        || authority.live_producer_manifest_sha256 != live_contract.live_producer_manifest_sha256
        || authority.owner_registry_manifest_sha256
            != live_contract.owner_registry_manifest_sha256
        || authority.selected_present_producer_manifest_sha256
            != live_contract.selected_present_producer_manifest_sha256
    {
        return Err(ObserverError::Unauthenticated(
            "observer authority does not match the pinned exact20 contract".into(),
        ));
    }
    let frame = source
        .observe_complete_transition(&raw)
        .map_err(ObserverError::Source)?;
    if frame.authority != authority
        || frame.owner_instance_id != expected_owner_instance_id(&raw.owner_registry_witness)
        || frame.frame_hour != raw.frame_hour
        || frame.elapsed_wall_ms != raw.elapsed_wall_ms
        || frame.capture_sequence != raw.capture_sequence
        || frame.callback_qpc_ticks != raw.callback_qpc_ticks
        || frame.boundary != raw.boundary
    {
        return invalid("bridge changed authority or callback transition identity");
    }
    encode_payload(&frame)
}

pub fn validate_exact20_contract(contract: Exact20HookContract) -> Result<(), ObserverError> {
    if !lower_hex_sha256(contract.exact20_receipt_sha256)
        || !lower_hex_sha256(contract.root_producer_manifest_sha256)
        || !lower_hex_sha256(contract.helper_producer_manifest_sha256)
    {
        return Err(ObserverError::Unauthenticated(
            "exact20 hook contract has an empty receipt or producer manifest".into(),
        ));
    }
    for (site, required_rva) in contract
        .function_sites
        .iter()
        .zip(REQUIRED_EXACT20_FUNCTION_RVAS)
    {
        if site.rva != required_rva || all_zero(&site.entry_sentinel) {
            return Err(ObserverError::Unauthenticated(
                "exact20 function set has a foreign RVA/order or empty sentinel".into(),
            ));
        }
    }
    Ok(())
}

pub fn validate_live_contract(
    contract: AuthenticatedLiveProducerContract,
) -> Result<(), ObserverError> {
    if !lower_hex_sha256(contract.live_observation_receipt_sha256)
        || !lower_hex_sha256(contract.live_producer_manifest_sha256)
        || !lower_hex_sha256(contract.owner_registry_manifest_sha256)
        || !lower_hex_sha256(contract.selected_present_producer_manifest_sha256)
        || contract.owner_hook.callback_rva != OWNER_CONSTRUCTOR_RVA
        || contract.population_hook.callback_rva != POPULATION_COMMIT_RVA
        || contract.refresh_hook.callback_rva != REFRESH_COMMIT_RVA
        || !lower_hex_sha256(contract.population_hook.installed_patch_sha256)
        || !lower_hex_sha256(contract.population_hook.trampoline_sha256)
        || !lower_hex_sha256(contract.refresh_hook.installed_patch_sha256)
        || !lower_hex_sha256(contract.refresh_hook.trampoline_sha256)
        || !lower_hex_sha256(contract.owner_hook.installed_patch_sha256)
        || !lower_hex_sha256(contract.owner_hook.trampoline_sha256)
    {
        return Err(ObserverError::Unauthenticated(
            "live producer contract is absent, malformed, or bound to foreign hooks".into(),
        ));
    }
    Ok(())
}

pub fn expected_owner_instance_id(witness: &OwnerRegistryWitness) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"the game:a729:technology-canvas-owner-registry:v1\0");
    put_u64(&mut canonical, witness.registry_epoch);
    put_u64(&mut canonical, witness.owner_sequence);
    put_u64(&mut canonical, witness.construction_qpc_ticks);
    put_len_prefixed_unchecked(
        &mut canonical,
        witness.owner_construction_record_sha256.as_bytes(),
    );
    put_len_prefixed_unchecked(&mut canonical, witness.registry_manifest_sha256.as_bytes());
    let digest = Sha256::digest(canonical);
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    format!("a729:technology_canvas_owner:{}", hex32(array))
}

pub fn expected_owner_construction_record_sha256(witness: &OwnerRegistryWitness) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"the game:a729:technology-canvas-owner-construction:v1\0");
    put_u64(&mut canonical, witness.registry_epoch);
    put_u64(&mut canonical, witness.owner_sequence);
    put_u64(&mut canonical, witness.construction_qpc_ticks);
    put_u32(&mut canonical, TECHNOLOGY_MODE);
    canonical.extend_from_slice(witness.registry_manifest_sha256.as_bytes());
    let digest = Sha256::digest(canonical);
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    hex32(array)
}

fn validate_raw_boundary(
    raw: &RawBoundaryObservation,
    contract: Exact20HookContract,
    live_contract: AuthenticatedLiveProducerContract,
) -> Result<(), ObserverError> {
    if !raw.boundary.is_complete_snapshot_boundary() {
        return invalid("owner construction is an identity boundary, not a complete snapshot");
    }
    if raw.owner_registry_witness.registry_epoch == 0
        || raw.owner_registry_witness.owner_sequence == 0
        || raw.owner_registry_witness.construction_qpc_ticks == 0
        || !lower_hex_sha256(
            &raw
                .owner_registry_witness
                .owner_construction_record_sha256,
        )
        || raw.owner_registry_witness.registry_manifest_sha256
            != live_contract.owner_registry_manifest_sha256
        || raw.owner_registry_witness.owner_construction_record_sha256
            != expected_owner_construction_record_sha256(&raw.owner_registry_witness)
    {
        return Err(ObserverError::Unauthenticated(
            "owner registry witness is absent or foreign".into(),
        ));
    }
    let callback_site = contract
        .function_sites
        .iter()
        .find(|site| site.rva == raw.callback_rva)
        .ok_or_else(|| {
            ObserverError::Unauthenticated("callback RVA is absent from exact20 contract".into())
        })?;
    let live_hook = match raw.boundary {
        ObserverBoundary::PopulationCommitted => live_contract.population_hook,
        ObserverBoundary::RefreshCommitted => live_contract.refresh_hook,
        ObserverBoundary::OwnerConstructed => {
            return invalid("owner construction cannot emit a complete snapshot")
        }
    };
    if callback_site.entry_sentinel != raw.original_entry_bytes_before_patch
        || raw.installed_patch_sha256 != live_hook.installed_patch_sha256
        || raw.trampoline_sha256 != live_hook.trampoline_sha256
    {
        return Err(ObserverError::Unauthenticated(
            "saved original, installed patch, or trampoline differs from contract".into(),
        ));
    }
    if raw.owner_root_ptr == 0
        || raw.controller_ptr == 0
        || raw.phase != ObservationPhase::PostReturn
        || raw.callback_rva != boundary_rva(raw.boundary)
        || raw.owner_mode6_slot_value != raw.controller_ptr
        || raw.controller_mode_raw != TECHNOLOGY_MODE
        || raw.capture_sequence == 0
        || raw.callback_qpc_ticks == 0
    {
        return invalid("mode-6 owner/controller/capture identity is not exact");
    }
    Ok(())
}

pub const fn boundary_rva(boundary: ObserverBoundary) -> usize {
    match boundary {
        ObserverBoundary::OwnerConstructed => OWNER_CONSTRUCTOR_RVA,
        ObserverBoundary::PopulationCommitted => POPULATION_COMMIT_RVA,
        ObserverBoundary::RefreshCommitted => REFRESH_COMMIT_RVA,
    }
}

pub fn validate_frame(frame: &TechnologyCanvasFrame) -> Result<(), ObserverError> {
    validate_authority(&frame.authority)?;
    nonempty(&frame.owner_instance_id, "owner_instance_id")?;
    nonempty(&frame.source_record_id, "source_record_id")?;
    if frame.boundary.is_complete_snapshot_boundary()
        && frame.source_record_id
            != expected_source_record_id(
                &frame.owner_instance_id,
                frame.frame_hour,
                frame.capture_sequence,
                frame.callback_qpc_ticks,
                frame.selected_present_candidate_id,
                frame.selected_present_ordinal,
                frame.selected_present_qpc_ticks,
                frame.selected_present_cutoff_qpc_ticks,
                frame.selected_present_engine_instance_token,
                frame.boundary,
                &frame.authored_surface_identity,
                &frame.runtime_root_identity,
                &frame.effective_interface_winner,
                &frame.active_root_identity,
                &frame.authority.selected_present_receipt_sha256,
                &frame.authority.selected_present_pass_identity_sha256,
                &frame.authority.selected_present_producer_manifest_sha256,
                &frame.authority.exact20_preflight_receipt_sha256,
                &frame.authority.root_producer_manifest_sha256,
                &frame.authority.helper_producer_manifest_sha256,
                &frame.authority.live_observation_receipt_sha256,
                &frame.authority.live_producer_manifest_sha256,
                &frame.authority.owner_registry_manifest_sha256,
                &frame.authority.content_manifest_sha256,
            )
    {
        return invalid("source_record_id does not bind owner/transition identity");
    }
    if !frame.boundary.is_complete_snapshot_boundary()
        || frame.controller_mode != TECHNOLOGY_MODE
        || frame.capture_sequence == 0
        || frame.callback_qpc_ticks == 0
        || frame.selected_present_candidate_id == 0
        || frame.selected_present_qpc_ticks < frame.callback_qpc_ticks
        || frame.selected_present_cutoff_qpc_ticks < frame.selected_present_qpc_ticks
        || frame.selected_present_engine_instance_token == 0
    {
        return invalid(
            "frame is not a complete mode-6 callback joined to a later selected Present",
        );
    }
    portable_canvas_atom(
        &frame.authored_surface_identity,
        "authored_surface_identity",
    )?;
    portable_canvas_atom(&frame.runtime_root_identity, "runtime_root_identity")?;
    portable_canvas_path(
        &frame.effective_interface_winner,
        "effective_interface_winner",
    )?;
    portable_canvas_path(&frame.active_root_identity, "active_root_identity")?;
    if frame.authored_surface_identity != "technology_canvas"
        || frame.runtime_root_identity != "countrytechtreeview"
        || frame.effective_interface_winner != "interface/countrytechtreeview.gui"
        || !frame
            .active_root_identity
            .starts_with("countrytechtreeview/")
    {
        return invalid("selected-Present technology root registration is foreign");
    }
    positive_rect(frame.clip_rect, "clip_rect")?;
    if frame.content_extent_pixels.iter().any(|value| *value <= 0)
        || frame.viewport_extent_pixels.iter().any(|value| *value <= 0)
        || frame
            .render_target_extent_pixels
            .iter()
            .any(|value| *value <= 0)
        || frame.client_extent_pixels.iter().any(|value| *value <= 0)
    {
        return invalid("content/viewport/render-target/client extents are invalid");
    }
    for (index, bits) in frame.canvas_scale_bits.iter().copied().enumerate() {
        let scale = f32::from_bits(bits);
        if !scale.is_finite() || scale <= 0.0 {
            return invalid(&format!("canvas scale component {index} is invalid"));
        }
    }
    validate_axis(frame.horizontal_scroll_fixed5, "horizontal scroll")?;
    validate_axis(frame.vertical_scroll_fixed5, "vertical scroll")?;
    let mapped_operation_count = frame
        .materialized_drawables
        .iter()
        .try_fold(0usize, |total, drawable| {
            total.checked_add(drawable.ui_pass_ordinals.len())
        })
        .ok_or_else(|| ObserverError::Invalid("UI operation count overflow".into()))?;
    if frame.source_items.len() > MAX_DRAWABLES
        || frame.final_draw_order.len() > MAX_DRAWABLES
        || frame.materialized_drawables.len() > MAX_DRAWABLES
        || frame.selected_present_ui_operation_order.len() > MAX_DRAWABLES
        || mapped_operation_count > MAX_DRAWABLES
    {
        return invalid("technology canvas exceeds carrier bounds");
    }
    if frame.source_items.is_empty()
        || frame.final_draw_order.is_empty()
        || frame.materialized_drawables.is_empty()
        || frame.selected_present_ui_operation_order.is_empty()
    {
        return invalid("complete technology canvas has an empty source/order/drawable set");
    }

    let mut source_ids = BTreeSet::new();
    let mut source_ordinals = BTreeSet::new();
    let mut hierarchy_paths = BTreeSet::new();
    let mut included = BTreeSet::new();
    let mut paths = BTreeMap::new();
    for (expected_ordinal, item) in frame.source_items.iter().enumerate() {
        portable_canvas_atom(&item.stable_item_id, "stable_item_id")?;
        portable_canvas_path(&item.hierarchy_path, "source hierarchy_path")?;
        portable_canvas_atom(&item.folder_id, "folder_id")?;
        if !source_ids.insert(item.stable_item_id.as_str())
            || !source_ordinals.insert(item.source_ordinal)
            || item.source_ordinal as usize != expected_ordinal
            || !hierarchy_paths.insert(item.hierarchy_path.as_str())
            || paths
                .insert(item.stable_item_id.as_str(), item.hierarchy_path.as_str())
                .is_some()
        {
            return invalid("duplicate source identity or ordinal");
        }
        if item.included {
            if item.exclusion_reason.is_some() {
                return invalid("included source item has an exclusion reason");
            }
            included.insert(item.stable_item_id.as_str());
        } else {
            nonempty(
                item.exclusion_reason.as_deref().unwrap_or(""),
                "excluded source reason",
            )?;
        }
    }

    let mut ordered = BTreeSet::new();
    for id in &frame.final_draw_order {
        portable_canvas_atom(id, "final draw id")?;
        if !ordered.insert(id.as_str()) || !included.contains(id.as_str()) {
            return invalid("final draw order contains a duplicate, absent, or excluded id");
        }
    }
    if ordered != included {
        return invalid("final draw order does not exactly cover included source items");
    }

    let mut drawables = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut mapped_ui_operations = BTreeSet::new();
    let mut previous_ui_operation = None;
    for (expected_ordinal, drawable) in frame.materialized_drawables.iter().enumerate() {
        portable_canvas_atom(&drawable.drawable_id, "drawable_id")?;
        portable_canvas_path(&drawable.hierarchy_path, "drawable hierarchy_path")?;
        portable_canvas_path(&drawable.slot_id, "slot_id")?;
        if drawable.draw_ordinal as usize != expected_ordinal
            || frame.final_draw_order.get(expected_ordinal) != Some(&drawable.drawable_id)
            || !drawables.insert(drawable.drawable_id.as_str())
            || !slots.insert(drawable.slot_id.as_str())
            || drawable.slot_id != drawable.hierarchy_path
            || paths.get(drawable.drawable_id.as_str()).copied()
                != Some(drawable.hierarchy_path.as_str())
        {
            return invalid("materialized drawable identity/order/path/slot mismatch");
        }
        positive_rect(drawable.rect, "materialized drawable rect")?;
        if drawable.ui_pass_ordinals.is_empty() {
            return invalid("materialized drawable has no selected-Present operations");
        }
        for operation_ordinal in &drawable.ui_pass_ordinals {
            if previous_ui_operation.is_some_and(|previous| previous >= *operation_ordinal)
                || !mapped_ui_operations.insert(*operation_ordinal)
            {
                return invalid(
                    "drawable selected-Present operations are duplicate or out of final order",
                );
            }
            previous_ui_operation = Some(*operation_ordinal);
        }
    }
    if drawables != included || frame.materialized_drawables.len() != frame.final_draw_order.len() {
        return invalid("materialized drawables do not exactly cover final order");
    }
    if frame
        .selected_present_ui_operation_order
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || frame.selected_present_ui_operation_order
            != mapped_ui_operations.iter().copied().collect::<Vec<_>>()
    {
        return invalid(
            "selected-Present operation order is not the exact ordered drawable union",
        );
    }
    if frame.authority.content_manifest_sha256 != compute_content_manifest_sha256(frame)? {
        return Err(ObserverError::Unauthenticated(
            "content manifest does not match the canonical complete frame".into(),
        ));
    }
    Ok(())
}

fn validate_authority(authority: &TechnologyCanvasAuthority) -> Result<(), ObserverError> {
    let contract = AUTHENTICATED_EXACT20_HOOK_CONTRACT.ok_or_else(|| {
        ObserverError::Unauthenticated("exact20 static authority is absent".into())
    })?;
    if authority.executable_sha256 != A729_EXECUTABLE_SHA256
        || !lower_hex_sha256(&authority.exact20_receipt_sha256)
        || !lower_hex_sha256(&authority.exact20_preflight_receipt_sha256)
        || !lower_hex_sha256(&authority.root_producer_manifest_sha256)
        || !lower_hex_sha256(&authority.helper_producer_manifest_sha256)
        || !lower_hex_sha256(&authority.live_observation_receipt_sha256)
        || !lower_hex_sha256(&authority.live_producer_manifest_sha256)
        || !lower_hex_sha256(&authority.owner_registry_manifest_sha256)
        || !lower_hex_sha256(&authority.selected_present_receipt_sha256)
        || !lower_hex_sha256(&authority.selected_present_pass_identity_sha256)
        || !lower_hex_sha256(&authority.selected_present_producer_manifest_sha256)
        || !lower_hex_sha256(&authority.content_manifest_sha256)
        || authority.exact20_receipt_sha256 != contract.exact20_receipt_sha256
        || authority.root_producer_manifest_sha256 != contract.root_producer_manifest_sha256
        || authority.helper_producer_manifest_sha256 != contract.helper_producer_manifest_sha256
    {
        return Err(ObserverError::Unauthenticated(
            "technology canvas authority hashes are absent, malformed, or foreign".into(),
        ));
    }
    Ok(())
}

fn validate_axis(axis: AxisStateFixed, name: &str) -> Result<(), ObserverError> {
    if axis.max < axis.min
        || !(axis.min..=axis.max).contains(&axis.current)
        || !(axis.min..=axis.max).contains(&axis.target)
    {
        invalid(&format!("{name} range/current/target is invalid"))
    } else {
        Ok(())
    }
}

fn encode_payload(frame: &TechnologyCanvasFrame) -> Result<Vec<u8>, ObserverError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u64(&mut out, frame.frame_hour);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.capture_sequence);
    put_u64(&mut out, frame.callback_qpc_ticks);
    put_u64(&mut out, frame.selected_present_candidate_id);
    put_u32(&mut out, frame.selected_present_ordinal);
    put_u64(&mut out, frame.selected_present_qpc_ticks);
    put_u64(&mut out, frame.selected_present_cutoff_qpc_ticks);
    put_u64(&mut out, frame.selected_present_engine_instance_token);
    put_u32(&mut out, frame.boundary as u32);
    put_u32(&mut out, frame.controller_mode);
    put_string(&mut out, &frame.authority.executable_sha256)?;
    put_string(&mut out, &frame.authority.exact20_receipt_sha256)?;
    put_string(&mut out, &frame.authority.exact20_preflight_receipt_sha256)?;
    put_string(&mut out, &frame.authority.root_producer_manifest_sha256)?;
    put_string(&mut out, &frame.authority.helper_producer_manifest_sha256)?;
    put_string(&mut out, &frame.authority.live_observation_receipt_sha256)?;
    put_string(&mut out, &frame.authority.live_producer_manifest_sha256)?;
    put_string(&mut out, &frame.authority.owner_registry_manifest_sha256)?;
    put_string(&mut out, &frame.authority.selected_present_receipt_sha256)?;
    put_string(
        &mut out,
        &frame.authority.selected_present_pass_identity_sha256,
    )?;
    put_string(
        &mut out,
        &frame.authority.selected_present_producer_manifest_sha256,
    )?;
    put_string(&mut out, &frame.authority.content_manifest_sha256)?;
    put_string(&mut out, &frame.source_record_id)?;
    put_string(&mut out, &frame.owner_instance_id)?;
    put_string(&mut out, &frame.authored_surface_identity)?;
    put_string(&mut out, &frame.runtime_root_identity)?;
    put_string(&mut out, &frame.effective_interface_winner)?;
    put_string(&mut out, &frame.active_root_identity)?;
    put_rect(&mut out, frame.clip_rect);
    for value in frame.content_extent_pixels {
        put_i32(&mut out, value);
    }
    for value in frame.viewport_extent_pixels {
        put_i32(&mut out, value);
    }
    for value in frame.render_target_extent_pixels {
        put_i32(&mut out, value);
    }
    for value in frame.client_extent_pixels {
        put_i32(&mut out, value);
    }
    for value in frame.canvas_origin_pixels_fixed5 {
        put_i64(&mut out, value);
    }
    for value in frame.canvas_scale_bits {
        put_u32(&mut out, value);
    }
    for value in frame.pan_pixels_fixed5 {
        put_i64(&mut out, value);
    }
    put_axis(&mut out, frame.horizontal_scroll_fixed5);
    put_axis(&mut out, frame.vertical_scroll_fixed5);
    put_u32(&mut out, frame.source_items.len() as u32);
    put_u32(&mut out, frame.final_draw_order.len() as u32);
    put_u32(&mut out, frame.materialized_drawables.len() as u32);
    put_u32(
        &mut out,
        frame.selected_present_ui_operation_order.len() as u32,
    );
    for item in &frame.source_items {
        put_string(&mut out, &item.stable_item_id)?;
        put_string(&mut out, &item.hierarchy_path)?;
        put_string(&mut out, &item.folder_id)?;
        put_u32(&mut out, item.source_ordinal);
        put_u32(&mut out, item.kind as u32);
        put_u32(&mut out, if item.included { 1 } else { 0 });
        put_optional_string(&mut out, item.exclusion_reason.as_deref())?;
    }
    for id in &frame.final_draw_order {
        put_string(&mut out, id)?;
    }
    for operation_ordinal in &frame.selected_present_ui_operation_order {
        put_u32(&mut out, *operation_ordinal);
    }
    for drawable in &frame.materialized_drawables {
        put_string(&mut out, &drawable.drawable_id)?;
        put_string(&mut out, &drawable.hierarchy_path)?;
        put_string(&mut out, &drawable.slot_id)?;
        put_u32(&mut out, drawable.draw_ordinal);
        put_rect(&mut out, drawable.rect);
        put_u32(&mut out, drawable.ui_pass_ordinals.len() as u32);
        for operation_ordinal in &drawable.ui_pass_ordinals {
            put_u32(&mut out, *operation_ordinal);
        }
    }
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<TechnologyCanvasFrame, ObserverError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("technology canvas magic/schema mismatch");
    }
    let frame_hour = reader.u64()?;
    let elapsed_wall_ms = reader.u64()?;
    let capture_sequence = reader.u64()?;
    let callback_qpc_ticks = reader.u64()?;
    let selected_present_candidate_id = reader.u64()?;
    let selected_present_ordinal = reader.u32()?;
    let selected_present_qpc_ticks = reader.u64()?;
    let selected_present_cutoff_qpc_ticks = reader.u64()?;
    let selected_present_engine_instance_token = reader.u64()?;
    let boundary = ObserverBoundary::from_wire(reader.u32()?)?;
    let controller_mode = reader.u32()?;
    let authority = TechnologyCanvasAuthority {
        executable_sha256: reader.string()?,
        exact20_receipt_sha256: reader.string()?,
        exact20_preflight_receipt_sha256: reader.string()?,
        root_producer_manifest_sha256: reader.string()?,
        helper_producer_manifest_sha256: reader.string()?,
        live_observation_receipt_sha256: reader.string()?,
        live_producer_manifest_sha256: reader.string()?,
        owner_registry_manifest_sha256: reader.string()?,
        selected_present_receipt_sha256: reader.string()?,
        selected_present_pass_identity_sha256: reader.string()?,
        selected_present_producer_manifest_sha256: reader.string()?,
        content_manifest_sha256: reader.string()?,
    };
    let source_record_id = reader.string()?;
    let owner_instance_id = reader.string()?;
    let authored_surface_identity = reader.string()?;
    let runtime_root_identity = reader.string()?;
    let effective_interface_winner = reader.string()?;
    let active_root_identity = reader.string()?;
    let clip_rect = reader.rect()?;
    let content_extent_pixels = [reader.i32()?, reader.i32()?];
    let viewport_extent_pixels = [reader.i32()?, reader.i32()?];
    let render_target_extent_pixels = [reader.i32()?, reader.i32()?];
    let client_extent_pixels = [reader.i32()?, reader.i32()?];
    let canvas_origin_pixels_fixed5 = [reader.i64()?, reader.i64()?];
    let canvas_scale_bits = [reader.u32()?, reader.u32()?];
    let pan_pixels_fixed5 = [reader.i64()?, reader.i64()?];
    let horizontal_scroll_fixed5 = reader.axis()?;
    let vertical_scroll_fixed5 = reader.axis()?;
    let source_count = bounded(reader.u32()?)?;
    let order_count = bounded(reader.u32()?)?;
    let drawable_count = bounded(reader.u32()?)?;
    let operation_count = bounded(reader.u32()?)?;
    let mut source_items = Vec::with_capacity(source_count);
    for _ in 0..source_count {
        source_items.push(TechnologySourceItem {
            stable_item_id: reader.string()?,
            hierarchy_path: reader.string()?,
            folder_id: reader.string()?,
            source_ordinal: reader.u32()?,
            kind: DrawableKind::from_wire(reader.u32()?)?,
            included: match reader.u32()? {
                0 => false,
                1 => true,
                _ => return invalid("included flag is not boolean"),
            },
            exclusion_reason: reader.optional_string()?,
        });
    }
    let mut final_draw_order = Vec::with_capacity(order_count);
    for _ in 0..order_count {
        final_draw_order.push(reader.string()?);
    }
    let mut selected_present_ui_operation_order = Vec::with_capacity(operation_count);
    for _ in 0..operation_count {
        selected_present_ui_operation_order.push(reader.u32()?);
    }
    let mut materialized_drawables = Vec::with_capacity(drawable_count);
    let mut decoded_operation_count = 0usize;
    for _ in 0..drawable_count {
        let drawable_id = reader.string()?;
        let hierarchy_path = reader.string()?;
        let slot_id = reader.string()?;
        let draw_ordinal = reader.u32()?;
        let rect = reader.rect()?;
        let count = bounded(reader.u32()?)?;
        decoded_operation_count = decoded_operation_count
            .checked_add(count)
            .filter(|total| *total <= MAX_DRAWABLES)
            .ok_or_else(|| {
                ObserverError::Invalid("decoded UI operation count exceeds bound".into())
            })?;
        let mut ui_pass_ordinals = Vec::with_capacity(count);
        for _ in 0..count {
            ui_pass_ordinals.push(reader.u32()?);
        }
        materialized_drawables.push(MaterializedDrawable {
            drawable_id,
            hierarchy_path,
            slot_id,
            draw_ordinal,
            rect,
            ui_pass_ordinals,
        });
    }
    if !reader.done() {
        return Err(ObserverError::TrailingBytes);
    }
    let frame = TechnologyCanvasFrame {
        authority,
        source_record_id,
        owner_instance_id,
        frame_hour,
        elapsed_wall_ms,
        capture_sequence,
        callback_qpc_ticks,
        selected_present_candidate_id,
        selected_present_ordinal,
        selected_present_qpc_ticks,
        selected_present_cutoff_qpc_ticks,
        selected_present_engine_instance_token,
        boundary,
        controller_mode,
        authored_surface_identity,
        runtime_root_identity,
        effective_interface_winner,
        active_root_identity,
        source_items,
        final_draw_order,
        selected_present_ui_operation_order,
        materialized_drawables,
        clip_rect,
        content_extent_pixels,
        viewport_extent_pixels,
        render_target_extent_pixels,
        client_extent_pixels,
        canvas_origin_pixels_fixed5,
        canvas_scale_bits,
        pan_pixels_fixed5,
        horizontal_scroll_fixed5,
        vertical_scroll_fixed5,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

/// Production-facing decoder. Structural decoding remains available for
/// dormant fixtures, but authenticated consumers must use this entrypoint.
pub fn decode_authenticated_payload(
    payload: &[u8],
) -> Result<TechnologyCanvasFrame, ObserverError> {
    if !PRODUCTION_ACTIVATION {
        return Err(ObserverError::Inactive(
            "H-A29 technology canvas observer is compile-time inactive",
        ));
    }
    let persisted_preflight = PERSISTED_EXACT20_PREFLIGHT_RECEIPT_SHA256.ok_or(
        ObserverError::Inactive("H-A29 persisted exact20 preflight receipt is absent"),
    )?;
    let live_contract = AUTHENTICATED_LIVE_PRODUCER_CONTRACT.ok_or(
        ObserverError::Inactive("H-A29 authenticated live producer contract is absent"),
    )?;
    validate_live_contract(live_contract)?;
    let frame = decode_payload(payload)?;
    if frame.authority.exact20_preflight_receipt_sha256 != persisted_preflight
        || frame.authority.live_observation_receipt_sha256
            != live_contract.live_observation_receipt_sha256
        || frame.authority.live_producer_manifest_sha256
            != live_contract.live_producer_manifest_sha256
        || frame.authority.owner_registry_manifest_sha256
            != live_contract.owner_registry_manifest_sha256
        || frame.authority.selected_present_producer_manifest_sha256
            != live_contract.selected_present_producer_manifest_sha256
    {
        return Err(ObserverError::Unauthenticated(
            "carrier live authority differs from pinned producer contract".into(),
        ));
    }
    Ok(frame)
}

/// `TCV1` is transported as a first-class domain-71 payload rather than being
/// projected into lossy `DCT1` collection rows. Kept as a mechanical promotion
/// check for the hook installer and recorder integration.
pub fn d71_projection_blockers() -> [&'static str; 0] {
    []
}

/// Computes the canonical callback-to-selected-Present content manifest.
/// `content_manifest_sha256` and `source_record_id` are excluded to avoid
/// circularity. The selected-Present receipt/producer hashes and every portable
/// observed canvas field are included.
pub fn compute_content_manifest_sha256(
    frame: &TechnologyCanvasFrame,
) -> Result<String, ObserverError> {
    let mapped_operation_count = frame
        .materialized_drawables
        .iter()
        .try_fold(0usize, |total, drawable| {
            total.checked_add(drawable.ui_pass_ordinals.len())
        })
        .ok_or_else(|| ObserverError::Invalid("UI operation count overflow".into()))?;
    if frame.source_items.len() > MAX_DRAWABLES
        || frame.final_draw_order.len() > MAX_DRAWABLES
        || frame.materialized_drawables.len() > MAX_DRAWABLES
        || frame.selected_present_ui_operation_order.len() > MAX_DRAWABLES
        || mapped_operation_count > MAX_DRAWABLES
    {
        return invalid("content manifest input exceeds carrier bounds");
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"the game:a729:technology-canvas-content:v2\0");
    put_string(&mut bytes, &frame.owner_instance_id)?;
    put_u64(&mut bytes, frame.frame_hour);
    put_u64(&mut bytes, frame.elapsed_wall_ms);
    put_u64(&mut bytes, frame.capture_sequence);
    put_u64(&mut bytes, frame.callback_qpc_ticks);
    put_u64(&mut bytes, frame.selected_present_candidate_id);
    put_u32(&mut bytes, frame.selected_present_ordinal);
    put_u64(&mut bytes, frame.selected_present_qpc_ticks);
    put_u64(&mut bytes, frame.selected_present_cutoff_qpc_ticks);
    put_u64(
        &mut bytes,
        frame.selected_present_engine_instance_token,
    );
    put_u32(&mut bytes, frame.boundary as u32);
    put_u32(&mut bytes, frame.controller_mode);
    put_string(
        &mut bytes,
        &frame.authority.selected_present_receipt_sha256,
    )?;
    put_string(
        &mut bytes,
        &frame.authority.selected_present_pass_identity_sha256,
    )?;
    put_string(
        &mut bytes,
        &frame.authority.selected_present_producer_manifest_sha256,
    )?;
    put_string(&mut bytes, &frame.authored_surface_identity)?;
    put_string(&mut bytes, &frame.runtime_root_identity)?;
    put_string(&mut bytes, &frame.effective_interface_winner)?;
    put_string(&mut bytes, &frame.active_root_identity)?;
    put_rect(&mut bytes, frame.clip_rect);
    for value in frame.content_extent_pixels {
        put_i32(&mut bytes, value);
    }
    for value in frame.viewport_extent_pixels {
        put_i32(&mut bytes, value);
    }
    for value in frame.render_target_extent_pixels {
        put_i32(&mut bytes, value);
    }
    for value in frame.client_extent_pixels {
        put_i32(&mut bytes, value);
    }
    for value in frame.canvas_origin_pixels_fixed5 {
        put_i64(&mut bytes, value);
    }
    for value in frame.canvas_scale_bits {
        put_u32(&mut bytes, value);
    }
    for value in frame.pan_pixels_fixed5 {
        put_i64(&mut bytes, value);
    }
    put_axis(&mut bytes, frame.horizontal_scroll_fixed5);
    put_axis(&mut bytes, frame.vertical_scroll_fixed5);
    put_u32(&mut bytes, frame.source_items.len() as u32);
    for item in &frame.source_items {
        put_string(&mut bytes, &item.stable_item_id)?;
        put_string(&mut bytes, &item.hierarchy_path)?;
        put_string(&mut bytes, &item.folder_id)?;
        put_u32(&mut bytes, item.source_ordinal);
        put_u32(&mut bytes, item.kind as u32);
        put_u32(&mut bytes, if item.included { 1 } else { 0 });
        put_optional_string(&mut bytes, item.exclusion_reason.as_deref())?;
    }
    put_u32(&mut bytes, frame.final_draw_order.len() as u32);
    for id in &frame.final_draw_order {
        put_string(&mut bytes, id)?;
    }
    put_u32(
        &mut bytes,
        frame.selected_present_ui_operation_order.len() as u32,
    );
    for operation_ordinal in &frame.selected_present_ui_operation_order {
        put_u32(&mut bytes, *operation_ordinal);
    }
    put_u32(&mut bytes, frame.materialized_drawables.len() as u32);
    for drawable in &frame.materialized_drawables {
        put_string(&mut bytes, &drawable.drawable_id)?;
        put_string(&mut bytes, &drawable.hierarchy_path)?;
        put_string(&mut bytes, &drawable.slot_id)?;
        put_u32(&mut bytes, drawable.draw_ordinal);
        put_rect(&mut bytes, drawable.rect);
        put_u32(&mut bytes, drawable.ui_pass_ordinals.len() as u32);
        for operation_ordinal in &drawable.ui_pass_ordinals {
            put_u32(&mut bytes, *operation_ordinal);
        }
    }
    let digest = Sha256::digest(bytes);
    let mut array = [0u8; 32];
    array.copy_from_slice(&digest);
    Ok(hex32(array))
}

fn lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().any(|byte| byte != b'0')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn all_zero<const N: usize>(value: &[u8; N]) -> bool {
    value.iter().all(|byte| *byte == 0)
}

fn hex32(value: [u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in value {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn nonempty(value: &str, field: &str) -> Result<(), ObserverError> {
    if value.trim().is_empty() || value.len() > MAX_STR {
        invalid(&format!("{field} is empty or oversized"))
    } else {
        Ok(())
    }
}

/// Portable authored/runtime token used for technology, folder, drawable, and
/// final-order identities. This deliberately supports the punctuation used by
/// exact connector identities (`edge:a->b:0`) and common mod identifiers while
/// excluding separators, whitespace, pointer spellings, and host paths.
fn portable_canvas_atom(value: &str, field: &str) -> Result<(), ObserverError> {
    let bytes = value.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= MAX_STR
        && (bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'@' | b':' | b'>')
        })
        && !value.contains("..")
        && !looks_like_pointer(value);
    if valid {
        Ok(())
    } else {
        invalid(&format!("{field} is not a portable canvas identity"))
    }
}

/// Canonical logical hierarchy/slot path. `/` is the only separator and every
/// component is itself a portable canvas atom; absolute, drive, UNC, empty,
/// dot, and parent components therefore fail closed.
fn portable_canvas_path(value: &str, field: &str) -> Result<(), ObserverError> {
    if value.len() > MAX_STR
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.split('/').any(|part| {
            matches!(part, "" | "." | "..") || portable_canvas_atom(part, field).is_err()
        })
    {
        invalid(&format!("{field} is not a canonical portable canvas path"))
    } else {
        Ok(())
    }
}

fn looks_like_pointer(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .is_some_and(|digits| {
            digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn positive_rect(rect: RectI32, field: &str) -> Result<(), ObserverError> {
    if rect.width <= 0
        || rect.height <= 0
        || rect.x.checked_add(rect.width).is_none()
        || rect.y.checked_add(rect.height).is_none()
    {
        invalid(&format!("{field} has nonpositive or overflowing extent"))
    } else {
        Ok(())
    }
}

fn invalid<T>(message: &str) -> Result<T, ObserverError> {
    Err(ObserverError::Invalid(message.into()))
}

fn bounded(value: u32) -> Result<usize, ObserverError> {
    let value = value as usize;
    if value > MAX_DRAWABLES {
        invalid("carrier count exceeds bound")
    } else {
        Ok(value)
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_len_prefixed_unchecked(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value);
}
fn put_rect(out: &mut Vec<u8>, rect: RectI32) {
    put_i32(out, rect.x);
    put_i32(out, rect.y);
    put_i32(out, rect.width);
    put_i32(out, rect.height);
}
fn put_axis(out: &mut Vec<u8>, axis: AxisStateFixed) {
    put_i64(out, axis.min);
    put_i64(out, axis.max);
    put_i64(out, axis.current);
    put_i64(out, axis.target);
}
fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), ObserverError> {
    nonempty(value, "wire string")?;
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), ObserverError> {
    match value {
        None => put_u32(out, u32::MAX),
        Some(value) => {
            nonempty(value, "optional wire string")?;
            put_u32(out, value.len() as u32);
            out.extend_from_slice(value.as_bytes());
        }
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], ObserverError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(ObserverError::Truncated)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(ObserverError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, ObserverError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, ObserverError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ObserverError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, ObserverError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn rect(&mut self) -> Result<RectI32, ObserverError> {
        Ok(RectI32 {
            x: self.i32()?,
            y: self.i32()?,
            width: self.i32()?,
            height: self.i32()?,
        })
    }
    fn axis(&mut self) -> Result<AxisStateFixed, ObserverError> {
        Ok(AxisStateFixed {
            min: self.i64()?,
            max: self.i64()?,
            current: self.i64()?,
            target: self.i64()?,
        })
    }
    fn string(&mut self) -> Result<String, ObserverError> {
        let count = self.u32()? as usize;
        if count == 0 || count > MAX_STR {
            return invalid("wire string length is invalid");
        }
        String::from_utf8(self.take(count)?.to_vec())
            .map_err(|_| ObserverError::Invalid("wire string is not UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, ObserverError> {
        let count = self.u32()?;
        if count == u32::MAX {
            return Ok(None);
        }
        let count = count as usize;
        if count == 0 || count > MAX_STR {
            return invalid("optional string length is invalid");
        }
        Ok(Some(
            String::from_utf8(self.take(count)?.to_vec())
                .map_err(|_| ObserverError::Invalid("optional string is not UTF-8".into()))?,
        ))
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn owner_witness() -> OwnerRegistryWitness {
        let mut witness = OwnerRegistryWitness {
            registry_epoch: 4,
            owner_sequence: 9,
            construction_qpc_ticks: 8_000,
            owner_construction_record_sha256: String::new(),
            registry_manifest_sha256: hash('f'),
        };
        witness.owner_construction_record_sha256 =
            expected_owner_construction_record_sha256(&witness);
        witness
    }

    fn complete() -> TechnologyCanvasFrame {
        let owner_instance_id = expected_owner_instance_id(&owner_witness());
        let boundary = ObserverBoundary::PopulationCommitted;
        let mut frame = TechnologyCanvasFrame {
            authority: TechnologyCanvasAuthority {
                executable_sha256: A729_EXECUTABLE_SHA256.into(),
                exact20_receipt_sha256: AUTHENTICATED_EXACT20_HOOK_CONTRACT
                    .unwrap()
                    .exact20_receipt_sha256
                    .into(),
                exact20_preflight_receipt_sha256: hash('3'),
                root_producer_manifest_sha256: AUTHENTICATED_EXACT20_HOOK_CONTRACT
                    .unwrap()
                    .root_producer_manifest_sha256
                    .into(),
                helper_producer_manifest_sha256: AUTHENTICATED_EXACT20_HOOK_CONTRACT
                    .unwrap()
                    .helper_producer_manifest_sha256
                    .into(),
                live_observation_receipt_sha256: hash('1'),
                live_producer_manifest_sha256: hash('2'),
                owner_registry_manifest_sha256: owner_witness().registry_manifest_sha256,
                selected_present_receipt_sha256: hash('4'),
                selected_present_pass_identity_sha256: hash('5'),
                selected_present_producer_manifest_sha256: hash('6'),
                content_manifest_sha256: hash('c'),
            },
            source_record_id: String::new(),
            owner_instance_id,
            frame_hour: 900,
            elapsed_wall_ms: 12,
            capture_sequence: 7,
            callback_qpc_ticks: 10_000,
            selected_present_candidate_id: 55,
            selected_present_ordinal: 3,
            selected_present_qpc_ticks: 10_100,
            selected_present_cutoff_qpc_ticks: 10_200,
            selected_present_engine_instance_token: 0x71,
            boundary,
            controller_mode: TECHNOLOGY_MODE,
            authored_surface_identity: "technology_canvas".into(),
            runtime_root_identity: "countrytechtreeview".into(),
            effective_interface_winner: "interface/countrytechtreeview.gui".into(),
            active_root_identity: "countrytechtreeview/infantry".into(),
            source_items: vec![
                TechnologySourceItem {
                    stable_item_id: "tech:infantry_weapons".into(),
                    hierarchy_path: "countrytechtreeview/infantry/infantry_weapons".into(),
                    folder_id: "infantry".into(),
                    source_ordinal: 0,
                    kind: DrawableKind::TechnologyNode,
                    included: true,
                    exclusion_reason: None,
                },
                TechnologySourceItem {
                    stable_item_id: "edge:infantry_weapons->support_weapons:0".into(),
                    hierarchy_path: "countrytechtreeview/infantry/edge:infantry_weapons->support_weapons/segment:0".into(),
                    folder_id: "infantry".into(),
                    source_ordinal: 1,
                    kind: DrawableKind::ConnectorSegment,
                    included: true,
                    exclusion_reason: None,
                },
                TechnologySourceItem {
                    stable_item_id: "tech:hidden_fixture".into(),
                    hierarchy_path: "countrytechtreeview/infantry/hidden_fixture".into(),
                    folder_id: "infantry".into(),
                    source_ordinal: 2,
                    kind: DrawableKind::TechnologyNode,
                    included: false,
                    exclusion_reason: Some("controller_visibility_predicate_false".into()),
                },
            ],
            final_draw_order: vec![
                "tech:infantry_weapons".into(),
                "edge:infantry_weapons->support_weapons:0".into(),
            ],
            selected_present_ui_operation_order: vec![8, 9, 10],
            materialized_drawables: vec![
                MaterializedDrawable {
                    drawable_id: "tech:infantry_weapons".into(),
                    hierarchy_path: "countrytechtreeview/infantry/infantry_weapons".into(),
                    slot_id: "countrytechtreeview/infantry/infantry_weapons".into(),
                    draw_ordinal: 0,
                    rect: RectI32 {
                        x: 20,
                        y: 30,
                        width: 183,
                        height: 84,
                    },
                    ui_pass_ordinals: vec![8, 9],
                },
                MaterializedDrawable {
                    drawable_id: "edge:infantry_weapons->support_weapons:0".into(),
                    hierarchy_path: "countrytechtreeview/infantry/edge:infantry_weapons->support_weapons/segment:0".into(),
                    slot_id: "countrytechtreeview/infantry/edge:infantry_weapons->support_weapons/segment:0".into(),
                    draw_ordinal: 1,
                    rect: RectI32 {
                        x: 190,
                        y: 50,
                        width: 70,
                        height: 70,
                    },
                    ui_pass_ordinals: vec![10],
                },
            ],
            clip_rect: RectI32 {
                x: 10,
                y: 20,
                width: 1000,
                height: 700,
            },
            content_extent_pixels: [1800, 1400],
            viewport_extent_pixels: [1000, 700],
            render_target_extent_pixels: [1920, 1080],
            client_extent_pixels: [1920, 1080],
            canvas_origin_pixels_fixed5: [320_000, 640_000],
            canvas_scale_bits: [1.0f32.to_bits(), 1.0f32.to_bits()],
            pan_pixels_fixed5: [-4_000_000, 12_500_000],
            horizontal_scroll_fixed5: AxisStateFixed {
                min: 0,
                max: 100_000_000,
                current: 4_000_000,
                target: 4_000_000,
            },
            vertical_scroll_fixed5: AxisStateFixed {
                min: 0,
                max: 100_000_000,
                current: 12_500_000,
                target: 12_500_000,
            },
        };
        frame.authority.content_manifest_sha256 =
            compute_content_manifest_sha256(&frame).unwrap();
        frame.source_record_id = expected_source_record_id(
            &frame.owner_instance_id,
            frame.frame_hour,
            frame.capture_sequence,
            frame.callback_qpc_ticks,
            frame.selected_present_candidate_id,
            frame.selected_present_ordinal,
            frame.selected_present_qpc_ticks,
            frame.selected_present_cutoff_qpc_ticks,
            frame.selected_present_engine_instance_token,
            frame.boundary,
            &frame.authored_surface_identity,
            &frame.runtime_root_identity,
            &frame.effective_interface_winner,
            &frame.active_root_identity,
            &frame.authority.selected_present_receipt_sha256,
            &frame.authority.selected_present_pass_identity_sha256,
            &frame.authority.selected_present_producer_manifest_sha256,
            &frame.authority.exact20_preflight_receipt_sha256,
            &frame.authority.root_producer_manifest_sha256,
            &frame.authority.helper_producer_manifest_sha256,
            &frame.authority.live_observation_receipt_sha256,
            &frame.authority.live_producer_manifest_sha256,
            &frame.authority.owner_registry_manifest_sha256,
            &frame.authority.content_manifest_sha256,
        );
        frame
    }

    #[test]
    fn pointer_free_complete_carrier_round_trips() {
        let frame = complete();
        let encoded = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), frame);
    }

    #[test]
    fn missing_drawable_or_manifest_rejects() {
        let mut missing = complete();
        missing.materialized_drawables.pop();
        assert!(validate_frame(&missing).is_err());

        let mut foreign = complete();
        foreign.authority.root_producer_manifest_sha256.clear();
        assert!(validate_frame(&foreign).is_err());
    }

    #[test]
    fn source_record_binds_owner_and_transition() {
        let mut frame = complete();
        frame.source_record_id = expected_source_record_id(
            &frame.owner_instance_id,
            frame.frame_hour,
            frame.capture_sequence + 1,
            frame.callback_qpc_ticks,
            frame.selected_present_candidate_id,
            frame.selected_present_ordinal,
            frame.selected_present_qpc_ticks,
            frame.selected_present_cutoff_qpc_ticks,
            frame.selected_present_engine_instance_token,
            frame.boundary,
            &frame.authored_surface_identity,
            &frame.runtime_root_identity,
            &frame.effective_interface_winner,
            &frame.active_root_identity,
            &frame.authority.selected_present_receipt_sha256,
            &frame.authority.selected_present_pass_identity_sha256,
            &frame.authority.selected_present_producer_manifest_sha256,
            &frame.authority.exact20_preflight_receipt_sha256,
            &frame.authority.root_producer_manifest_sha256,
            &frame.authority.helper_producer_manifest_sha256,
            &frame.authority.live_observation_receipt_sha256,
            &frame.authority.live_producer_manifest_sha256,
            &frame.authority.owner_registry_manifest_sha256,
            &frame.authority.content_manifest_sha256,
        );
        assert!(validate_frame(&frame).is_err());
    }

    struct Fake;
    impl private::Sealed for Fake {}
    impl ExactTechnologyCanvasObserverSource for Fake {
        fn authority(&self) -> TechnologyCanvasAuthority {
            complete().authority
        }

        fn observe_complete_transition(
            &self,
            _raw: &RawBoundaryObservation,
        ) -> Result<TechnologyCanvasFrame, String> {
            Ok(complete())
        }
    }

    fn contract() -> Exact20HookContract {
        Exact20HookContract {
            exact20_receipt_sha256:
                "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
            root_producer_manifest_sha256:
                "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2",
            helper_producer_manifest_sha256:
                "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
            function_sites: std::array::from_fn(|index| FunctionSiteContract {
                rva: REQUIRED_EXACT20_FUNCTION_RVAS[index],
                entry_sentinel: [(index + 1) as u8; 16],
            }),
        }
    }

    fn live_contract() -> AuthenticatedLiveProducerContract {
        AuthenticatedLiveProducerContract {
            live_observation_receipt_sha256:
                "1111111111111111111111111111111111111111111111111111111111111111",
            live_producer_manifest_sha256:
                "2222222222222222222222222222222222222222222222222222222222222222",
            owner_registry_manifest_sha256:
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            selected_present_producer_manifest_sha256:
                "6666666666666666666666666666666666666666666666666666666666666666",
            owner_hook: LiveHookInstallationContract {
                callback_rva: OWNER_CONSTRUCTOR_RVA,
                installed_patch_sha256:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                trampoline_sha256:
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            },
            population_hook: LiveHookInstallationContract {
                callback_rva: POPULATION_COMMIT_RVA,
                installed_patch_sha256:
                    "7777777777777777777777777777777777777777777777777777777777777777",
                trampoline_sha256:
                    "8888888888888888888888888888888888888888888888888888888888888888",
            },
            refresh_hook: LiveHookInstallationContract {
                callback_rva: REFRESH_COMMIT_RVA,
                installed_patch_sha256:
                    "9999999999999999999999999999999999999999999999999999999999999999",
                trampoline_sha256:
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            },
        }
    }

    fn population_raw() -> RawBoundaryObservation {
        RawBoundaryObservation {
            callback_rva: POPULATION_COMMIT_RVA,
            original_entry_bytes_before_patch: [3; 16],
            installed_patch_sha256: hash('7'),
            trampoline_sha256: hash('8'),
            phase: ObservationPhase::PostReturn,
            owner_root_ptr: 1,
            controller_ptr: 2,
            owner_mode6_slot_value: 2,
            owner_registry_witness: owner_witness(),
            controller_mode_raw: TECHNOLOGY_MODE,
            frame_hour: 900,
            elapsed_wall_ms: 12,
            capture_sequence: 7,
            callback_qpc_ticks: 10_000,
            boundary: ObserverBoundary::PopulationCommitted,
        }
    }

    #[test]
    fn function_entry_is_not_a_complete_transition() {
        let mut raw = population_raw();
        raw.phase = ObservationPhase::FunctionEntry;
        assert!(validate_raw_boundary(&raw, contract(), live_contract()).is_err());
    }

    #[test]
    fn callback_bytes_and_all_exact20_roots_are_authenticated() {
        let mut raw = population_raw();
        raw.original_entry_bytes_before_patch[0] ^= 0xFF;
        assert!(validate_raw_boundary(&raw, contract(), live_contract()).is_err());

        let mut incomplete = contract();
        incomplete.function_sites[5].rva = 0;
        assert!(validate_exact20_contract(incomplete).is_err());
    }

    #[test]
    fn content_manifest_is_recomputed_from_complete_carrier() {
        let mut frame = complete();
        frame.materialized_drawables[0].rect.x += 1;
        assert!(validate_frame(&frame).is_err());
    }

    fn reseal(mut frame: TechnologyCanvasFrame) -> TechnologyCanvasFrame {
        frame.authority.content_manifest_sha256 = compute_content_manifest_sha256(&frame).unwrap();
        frame.source_record_id = expected_source_record_id(
            &frame.owner_instance_id,
            frame.frame_hour,
            frame.capture_sequence,
            frame.callback_qpc_ticks,
            frame.selected_present_candidate_id,
            frame.selected_present_ordinal,
            frame.selected_present_qpc_ticks,
            frame.selected_present_cutoff_qpc_ticks,
            frame.selected_present_engine_instance_token,
            frame.boundary,
            &frame.authored_surface_identity,
            &frame.runtime_root_identity,
            &frame.effective_interface_winner,
            &frame.active_root_identity,
            &frame.authority.selected_present_receipt_sha256,
            &frame.authority.selected_present_pass_identity_sha256,
            &frame.authority.selected_present_producer_manifest_sha256,
            &frame.authority.exact20_preflight_receipt_sha256,
            &frame.authority.root_producer_manifest_sha256,
            &frame.authority.helper_producer_manifest_sha256,
            &frame.authority.live_observation_receipt_sha256,
            &frame.authority.live_producer_manifest_sha256,
            &frame.authority.owner_registry_manifest_sha256,
            &frame.authority.content_manifest_sha256,
        );
        frame
    }

    #[test]
    fn canvas_identities_reject_pointer_and_host_path_mutations_even_when_resealed() {
        let mut atom = complete();
        atom.source_items[0].stable_item_id = "0x000001abcdef1234".into();
        atom.final_draw_order[0] = atom.source_items[0].stable_item_id.clone();
        atom.materialized_drawables[0].drawable_id = atom.source_items[0].stable_item_id.clone();
        assert!(validate_frame(&reseal(atom)).is_err());

        for bad_path in [
            r"C:\\hoi4\\countrytechtreeview\\infantry",
            "/opt/hoi4/countrytechtreeview/infantry",
            r"\\capture-host\\share\\countrytechtreeview",
            "countrytechtreeview/../infantry",
            "countrytechtreeview//infantry",
        ] {
            let mut frame = complete();
            frame.source_items[0].hierarchy_path = bad_path.into();
            frame.materialized_drawables[0].hierarchy_path = bad_path.into();
            frame.materialized_drawables[0].slot_id = bad_path.into();
            assert!(validate_frame(&reseal(frame)).is_err(), "{bad_path}");
        }
    }

    #[test]
    fn canvas_identity_grammar_preserves_mod_and_connector_tokens() {
        for atom in ["my_mod.tech-v2", "shared@folder", "edge:a->b:17"] {
            assert!(portable_canvas_atom(atom, "fixture").is_ok(), "{atom}");
        }
        assert!(portable_canvas_path(
            "countrytechtreeview/my_mod.folder/edge:a->b:17/segment:0",
            "fixture"
        )
        .is_ok());
    }

    #[test]
    fn owner_identity_is_derived_from_registry_witness() {
        let raw = population_raw();
        assert_eq!(
            complete().owner_instance_id,
            expected_owner_instance_id(&raw.owner_registry_witness)
        );
        assert!(decode_authenticated_payload(&encode_payload(&complete()).unwrap()).is_err());
    }

    #[test]
    fn valid_fake_cannot_emit_while_activation_is_false() {
        let raw = population_raw();
        assert!(matches!(
            collect_after_boundary(&Fake, raw),
            Err(ObserverError::Inactive(_))
        ));
        assert!(!activation_blockers().is_empty());
        assert!(d71_projection_blockers().is_empty());
    }
}
