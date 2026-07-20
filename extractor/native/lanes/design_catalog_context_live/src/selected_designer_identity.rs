//! A40/XUI08 selected-designer prerequisite carrier.
//!
//! This module is deliberately unscheduled. It defines the authenticated,
//! pointer-free boundary required before the six-family human opening matrix
//! can be useful. The static owner, selection and classification receipts are
//! now bound, but the live observer stays disabled until a six-family fixture
//! proves real invocation and selection transitions.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 85;
pub const DOMAIN_NAME: &str = "selected_designer_identity_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"SDI1");
pub const AUTHORITY_EXACT_A729_SELECTED_DESIGNER: u32 = 1;
/// Each latch flips only when its root-owned receipt names the final manifest.
/// The producer receipt is bound; the independent classifier receipt is not.
/// The pointer-free codec is usable for deterministic transport tests, but the
/// live collector seam cannot authenticate or emit a production frame yet.
pub const PROMOTION_AUTHORITY_BOUND: bool = true;
pub const CLASSIFIER_AUTHORITY_BOUND: bool = true;
/// Static authority is complete. Production collection still requires the
/// six-family live invocation/selection fixture and therefore stays disabled.
pub const LIVE_FIXTURE_AUTHORITY_BOUND: bool = false;
pub const PRODUCTION_ACTIVATION: bool = PROMOTION_AUTHORITY_BOUND
    && CLASSIFIER_AUTHORITY_BOUND
    && LIVE_FIXTURE_AUTHORITY_BOUND;
pub const EXACT_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
/// Root-owned exact20 receipt
/// `20260720T020813Z-w3-a40-designers-fc2ae305/shard-04`, 10/10 exact roots.
pub const PRODUCER_MANIFEST_SHA256: [u8; 32] = [
    0x6d, 0xff, 0x47, 0xe4, 0xfa, 0xe1, 0xf9, 0xd3, 0xae, 0xab, 0xb6, 0x3a, 0xc7, 0xe9, 0xa9, 0xcc,
    0x13, 0xa2, 0x7c, 0x42, 0x9d, 0x5e, 0x21, 0x7d, 0x68, 0x63, 0x11, 0xd5, 0xa3, 0x49, 0xcb, 0x6b,
];
/// Exact 10-root classifier/selection-chain function receipt
/// `20260720T022731Z-w3-a40-classifier-final-65c68d38/shard-09`.
pub const CLASSIFIER_MANIFEST_SHA256: [u8; 32] = [
    0x76, 0x68, 0x44, 0x86, 0x67, 0x8f, 0xdb, 0x87, 0xf4, 0xa6, 0x79, 0xc4, 0xfa, 0xa8, 0x55, 0xb6,
    0x1d, 0x45, 0x51, 0xc0, 0xa1, 0x3b, 0xeb, 0xbd, 0x6b, 0xa0, 0xc1, 0x55, 0x3d, 0xbf, 0x6a, 0x41,
];
/// Raw-byte authority for non-function division click thunk `0x141490EA0`.
pub const DIVISION_CLICK_THUNK_ARTIFACT_SHA256: [u8; 32] = [
    0x85, 0x6f, 0x8f, 0xab, 0xd2, 0x3a, 0xd5, 0x2b, 0x8a, 0xde, 0x50, 0x23, 0x46, 0xb0, 0x59, 0xae,
    0x9f, 0xf7, 0x6f, 0xdc, 0x9a, 0x9d, 0x90, 0x88, 0x14, 0xc9, 0x85, 0x90, 0x4e, 0x10, 0x04, 0x51,
];
/// Raw-byte authority for non-function wrapper `0x1417506D0`.
pub const DIVISION_WRAPPER_ARTIFACT_SHA256: [u8; 32] = [
    0x7d, 0xbc, 0xad, 0x84, 0x66, 0x16, 0x3a, 0x3f, 0xe1, 0xb0, 0xa3, 0x1c, 0xa3, 0x27, 0x66, 0x62,
    0xe6, 0x8f, 0xd7, 0xea, 0x39, 0x5e, 0x2d, 0x10, 0x2b, 0x9c, 0x44, 0xe0, 0x8d, 0xf5, 0xef, 0x0d,
];
pub const REFERENCED_DIVISION_TEMPLATE_TYPE: u32 = 52;
pub const EQUIPMENT_VARIANT_TYPE: u32 = 70;
pub const MAX_ORDERED_DESIGNS: usize = 131_072;
pub const MAX_SOURCE_RECORD_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectedDesignerFamily {
    Division,
    Equipment,
    Tank,
    Plane,
    Ship,
    VariantUpgrade,
}

impl SelectedDesignerFamily {
    pub const fn stable_object_type(self) -> u32 {
        match self {
            Self::Division => REFERENCED_DIVISION_TEMPLATE_TYPE,
            Self::Equipment
            | Self::Tank
            | Self::Plane
            | Self::Ship
            | Self::VariantUpgrade => EQUIPMENT_VARIANT_TYPE,
        }
    }
}

/// Exact live owner which supplied the selection. Equipment and ship share the
/// mode-0 owner and must be separated by the authenticated variant classifier;
/// there is no invented fourth `CEquipmentDesignerView` mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedDesignerOwner {
    CountryDivisionDesigner,
    CountryEquipmentDesignerMode0,
    TankDesignerMode1,
    PlaneDesignerMode2,
    EquipmentUpgradeDesignerWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StableDesignIdentity {
    /// Exact engine referenced-object type discriminator.
    pub object_type: u32,
    /// Positive stable id from the referenced object body, never an address.
    pub stable_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedDesignerIdentityFrame {
    pub frame_hour: u64,
    pub transition_sequence: u64,
    pub executable_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    pub classifier_manifest_sha256: [u8; 32],
    pub division_click_thunk_artifact_sha256: [u8; 32],
    pub division_wrapper_artifact_sha256: [u8; 32],
    pub source_record_identity: String,
    pub owner: SelectedDesignerOwner,
    pub family: SelectedDesignerFamily,
    pub expected_ordered_design_count: u32,
    /// Exact controller-published order. No numeric or token sort is allowed.
    pub ordered_designs: Vec<StableDesignIdentity>,
    pub selected_design: StableDesignIdentity,
    pub complete: bool,
}

/// Compact-domain envelope. A clean frame must contain a validated snapshot;
/// an unavailable frame must carry a nonzero compact failure status and no
/// snapshot. This distinction prevents an unavailable observer from being
/// replayed as an empty designer list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedDesignerTransportFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub snapshot: Option<SelectedDesignerIdentityFrame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedDesignerError {
    ActivationBlocked,
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactSelectedDesignerIdentitySource {
    fn is_exact_a729_selected_designer_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn classifier_manifest_sha256(&self) -> [u8; 32];
    fn collect_frame(
        &self,
        frame_hour: u64,
        family: SelectedDesignerFamily,
    ) -> Result<SelectedDesignerIdentityFrame, String>;
}

pub fn collect_and_validate<S: ExactSelectedDesignerIdentitySource>(
    source: &S,
    frame_hour: u64,
    family: SelectedDesignerFamily,
) -> Result<SelectedDesignerIdentityFrame, SelectedDesignerError> {
    if !PRODUCTION_ACTIVATION {
        return Err(SelectedDesignerError::ActivationBlocked);
    }
    if !source.is_exact_a729_selected_designer_source() {
        return Err(SelectedDesignerError::UnauthenticatedSource);
    }
    let frame = source
        .collect_frame(frame_hour, family)
        .map_err(SelectedDesignerError::Source)?;
    if frame.frame_hour != frame_hour || frame.family != family {
        return invalid("requested and returned designer snapshot identities differ");
    }
    if frame.producer_manifest_sha256 != source.producer_manifest_sha256()
        || frame.classifier_manifest_sha256 != source.classifier_manifest_sha256()
    {
        return invalid("designer frame/source authority manifest mismatch");
    }
    validate_frame(&frame)?;
    Ok(frame)
}

/// The only collector-facing live seam. It is structurally complete but
/// compile-time fail-closed while either authority receipt is absent.
pub fn collect_and_encode<S: ExactSelectedDesignerIdentitySource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    family: SelectedDesignerFamily,
) -> Result<Vec<u8>, SelectedDesignerError> {
    let snapshot = collect_and_validate(source, frame_hour, family)?;
    encode_payload(&SelectedDesignerTransportFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        snapshot: Some(snapshot),
    })
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, SelectedDesignerError> {
    if status_flags == 0 {
        return invalid("source-unavailable frame requires nonzero failure status");
    }
    encode_payload(&SelectedDesignerTransportFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        snapshot: None,
    })
}

pub fn validate_transport_frame(
    frame: &SelectedDesignerTransportFrame,
) -> Result<(), SelectedDesignerError> {
    match &frame.snapshot {
        Some(snapshot) if frame.status_flags == 0 && snapshot.frame_hour == frame.frame_hour => {
            validate_frame(snapshot)
        }
        Some(_) => invalid("clean selected-designer snapshot/status/hour invariant failed"),
        None if frame.status_flags != 0 => Ok(()),
        None => invalid("missing selected-designer snapshot requires failure status"),
    }
}

pub fn encode_payload(
    frame: &SelectedDesignerTransportFrame,
) -> Result<Vec<u8>, SelectedDesignerError> {
    validate_transport_frame(frame)?;
    let mut out = Vec::new();
    push_u32(&mut out, MAGIC);
    push_u32(&mut out, SCHEMA_VERSION);
    push_u32(&mut out, AUTHORITY_EXACT_A729_SELECTED_DESIGNER);
    push_u32(&mut out, frame.status_flags);
    push_u64(&mut out, frame.elapsed_wall_ms);
    push_u64(&mut out, frame.frame_hour);
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 7]);
    let Some(snapshot) = &frame.snapshot else {
        return Ok(out);
    };
    push_u64(&mut out, snapshot.transition_sequence);
    out.extend_from_slice(&snapshot.executable_sha256);
    out.extend_from_slice(&snapshot.producer_manifest_sha256);
    out.extend_from_slice(&snapshot.classifier_manifest_sha256);
    out.extend_from_slice(&snapshot.division_click_thunk_artifact_sha256);
    out.extend_from_slice(&snapshot.division_wrapper_artifact_sha256);
    out.push(family_raw(snapshot.family));
    out.push(owner_raw(snapshot.owner));
    out.push(u8::from(snapshot.complete));
    out.push(0);
    push_u32(&mut out, snapshot.expected_ordered_design_count);
    push_u32(&mut out, snapshot.ordered_designs.len() as u32);
    push_identity(&mut out, snapshot.selected_design);
    push_string(&mut out, &snapshot.source_record_identity)?;
    for identity in &snapshot.ordered_designs {
        push_identity(&mut out, *identity);
    }
    Ok(out)
}

pub fn decode_payload(
    payload: &[u8],
) -> Result<SelectedDesignerTransportFrame, SelectedDesignerError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC
        || reader.u32()? != SCHEMA_VERSION
        || reader.u32()? != AUTHORITY_EXACT_A729_SELECTED_DESIGNER
    {
        return invalid("SDI1 magic/schema/authority mismatch");
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let has_snapshot = reader.boolean()?;
    if reader.bytes(7)? != [0; 7] {
        return invalid("SDI1 availability reserved bytes are nonzero");
    }
    let snapshot = if has_snapshot {
        let transition_sequence = reader.u64()?;
        let executable_sha256 = reader.array32()?;
        let producer_manifest_sha256 = reader.array32()?;
        let classifier_manifest_sha256 = reader.array32()?;
        let division_click_thunk_artifact_sha256 = reader.array32()?;
        let division_wrapper_artifact_sha256 = reader.array32()?;
        let family = family_from_raw(reader.u8()?)?;
        let owner = owner_from_raw(reader.u8()?)?;
        let complete = reader.boolean()?;
        if reader.u8()? != 0 {
            return invalid("SDI1 snapshot reserved byte is nonzero");
        }
        let expected_ordered_design_count = reader.u32()?;
        let ordered_count = reader.u32()? as usize;
        if ordered_count > MAX_ORDERED_DESIGNS {
            return invalid("SDI1 ordered design count exceeds bound");
        }
        let selected_design = reader.identity()?;
        let source_record_identity = reader.string()?;
        let mut ordered_designs = Vec::with_capacity(ordered_count);
        for _ in 0..ordered_count {
            ordered_designs.push(reader.identity()?);
        }
        Some(SelectedDesignerIdentityFrame {
            frame_hour,
            transition_sequence,
            executable_sha256,
            producer_manifest_sha256,
            classifier_manifest_sha256,
            division_click_thunk_artifact_sha256,
            division_wrapper_artifact_sha256,
            source_record_identity,
            owner,
            family,
            expected_ordered_design_count,
            ordered_designs,
            selected_design,
            complete,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(SelectedDesignerError::TrailingBytes);
    }
    let frame = SelectedDesignerTransportFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        snapshot,
    };
    validate_transport_frame(&frame)?;
    Ok(frame)
}

pub fn validate_frame(
    frame: &SelectedDesignerIdentityFrame,
) -> Result<(), SelectedDesignerError> {
    if frame.executable_sha256 != EXACT_A729_EXECUTABLE_SHA256 {
        return invalid("selected-designer frame is not bound to exact a729 executable");
    }
    if frame.producer_manifest_sha256 != PRODUCER_MANIFEST_SHA256 {
        return invalid("selected-designer frame producer manifest is not the exact20 receipt");
    }
    if frame.classifier_manifest_sha256 != CLASSIFIER_MANIFEST_SHA256
        || frame.division_click_thunk_artifact_sha256
            != DIVISION_CLICK_THUNK_ARTIFACT_SHA256
        || frame.division_wrapper_artifact_sha256 != DIVISION_WRAPPER_ARTIFACT_SHA256
    {
        return invalid("selected-designer classifier/thunk authority receipt mismatch");
    }
    if !frame.complete {
        return invalid("selected-designer frame is incomplete");
    }
    if frame.transition_sequence == 0 {
        return invalid("selected-designer transition sequence is zero");
    }
    validate_nonempty(&frame.source_record_identity, "source record identity")?;
    if frame.producer_manifest_sha256 == [0; 32]
        || frame.classifier_manifest_sha256 == [0; 32]
    {
        return invalid("selected-designer authority manifest hash is zero");
    }
    validate_owner_family(frame.owner, frame.family)?;

    if frame.ordered_designs.is_empty()
        || frame.ordered_designs.len() > MAX_ORDERED_DESIGNS
        || frame.expected_ordered_design_count as usize != frame.ordered_designs.len()
    {
        return invalid("ordered-design completeness/count invariant failed");
    }

    let expected_type = frame.family.stable_object_type();
    let mut identities = BTreeSet::new();
    for identity in &frame.ordered_designs {
        validate_identity(*identity, expected_type)?;
        if !identities.insert(*identity) {
            return invalid("ordered designer rows contain a duplicate stable identity");
        }
    }
    validate_identity(frame.selected_design, expected_type)?;
    if !identities.contains(&frame.selected_design) {
        return invalid("selected design is absent from exact controller order");
    }
    Ok(())
}

fn validate_owner_family(
    owner: SelectedDesignerOwner,
    family: SelectedDesignerFamily,
) -> Result<(), SelectedDesignerError> {
    let valid = matches!(
        (owner, family),
        (
            SelectedDesignerOwner::CountryDivisionDesigner,
            SelectedDesignerFamily::Division
        ) | (
            SelectedDesignerOwner::CountryEquipmentDesignerMode0,
            SelectedDesignerFamily::Equipment | SelectedDesignerFamily::Ship
        ) | (
            SelectedDesignerOwner::TankDesignerMode1,
            SelectedDesignerFamily::Tank
        ) | (
            SelectedDesignerOwner::PlaneDesignerMode2,
            SelectedDesignerFamily::Plane
        ) | (
            SelectedDesignerOwner::EquipmentUpgradeDesignerWindow,
            SelectedDesignerFamily::VariantUpgrade
        )
    );
    if !valid {
        return invalid("designer owner and semantic family are inconsistent");
    }
    Ok(())
}

fn validate_identity(
    identity: StableDesignIdentity,
    expected_type: u32,
) -> Result<(), SelectedDesignerError> {
    if identity.object_type != expected_type || identity.stable_id == 0 {
        return invalid("design identity has wrong object type or zero stable id");
    }
    Ok(())
}

fn validate_nonempty(value: &str, label: &str) -> Result<(), SelectedDesignerError> {
    if value.trim().is_empty()
        || value.len() > MAX_SOURCE_RECORD_BYTES
        || value.chars().any(char::is_control)
    {
        return invalid(&format!(
            "{label} is empty, unbounded, or contains controls"
        ));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, SelectedDesignerError> {
    Err(SelectedDesignerError::Invalid(message.to_string()))
}

const fn family_raw(value: SelectedDesignerFamily) -> u8 {
    match value {
        SelectedDesignerFamily::Division => 0,
        SelectedDesignerFamily::Equipment => 1,
        SelectedDesignerFamily::Tank => 2,
        SelectedDesignerFamily::Plane => 3,
        SelectedDesignerFamily::Ship => 4,
        SelectedDesignerFamily::VariantUpgrade => 5,
    }
}

fn family_from_raw(value: u8) -> Result<SelectedDesignerFamily, SelectedDesignerError> {
    match value {
        0 => Ok(SelectedDesignerFamily::Division),
        1 => Ok(SelectedDesignerFamily::Equipment),
        2 => Ok(SelectedDesignerFamily::Tank),
        3 => Ok(SelectedDesignerFamily::Plane),
        4 => Ok(SelectedDesignerFamily::Ship),
        5 => Ok(SelectedDesignerFamily::VariantUpgrade),
        _ => invalid("SDI1 designer family is outside range 0..=5"),
    }
}

const fn owner_raw(value: SelectedDesignerOwner) -> u8 {
    match value {
        SelectedDesignerOwner::CountryDivisionDesigner => 0,
        SelectedDesignerOwner::CountryEquipmentDesignerMode0 => 1,
        SelectedDesignerOwner::TankDesignerMode1 => 2,
        SelectedDesignerOwner::PlaneDesignerMode2 => 3,
        SelectedDesignerOwner::EquipmentUpgradeDesignerWindow => 4,
    }
}

fn owner_from_raw(value: u8) -> Result<SelectedDesignerOwner, SelectedDesignerError> {
    match value {
        0 => Ok(SelectedDesignerOwner::CountryDivisionDesigner),
        1 => Ok(SelectedDesignerOwner::CountryEquipmentDesignerMode0),
        2 => Ok(SelectedDesignerOwner::TankDesignerMode1),
        3 => Ok(SelectedDesignerOwner::PlaneDesignerMode2),
        4 => Ok(SelectedDesignerOwner::EquipmentUpgradeDesignerWindow),
        _ => invalid("SDI1 designer owner is outside range 0..=4"),
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_identity(out: &mut Vec<u8>, value: StableDesignIdentity) {
    push_u32(out, value.object_type);
    push_u32(out, value.stable_id);
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), SelectedDesignerError> {
    validate_nonempty(value, "serialized source record identity")?;
    push_u32(
        out,
        u32::try_from(value.len())
            .map_err(|_| SelectedDesignerError::Invalid("SDI1 string is too long".into()))?,
    );
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], SelectedDesignerError> {
        let end = self
            .at
            .checked_add(len)
            .ok_or(SelectedDesignerError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(SelectedDesignerError::Truncated)?;
        self.at = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, SelectedDesignerError> {
        Ok(self.bytes(1)?[0])
    }

    fn boolean(&mut self) -> Result<bool, SelectedDesignerError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("SDI1 bool byte is outside range 0..=1"),
        }
    }

    fn u32(&mut self) -> Result<u32, SelectedDesignerError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, SelectedDesignerError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    fn array32(&mut self) -> Result<[u8; 32], SelectedDesignerError> {
        Ok(self.bytes(32)?.try_into().unwrap())
    }

    fn identity(&mut self) -> Result<StableDesignIdentity, SelectedDesignerError> {
        Ok(StableDesignIdentity {
            object_type: self.u32()?,
            stable_id: self.u32()?,
        })
    }

    fn string(&mut self) -> Result<String, SelectedDesignerError> {
        let len = self.u32()? as usize;
        if len > MAX_SOURCE_RECORD_BYTES {
            return invalid("SDI1 string exceeds byte bound");
        }
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| SelectedDesignerError::Invalid("SDI1 string is not UTF-8".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(id: u32) -> StableDesignIdentity {
        StableDesignIdentity {
            object_type: EQUIPMENT_VARIANT_TYPE,
            stable_id: id,
        }
    }

    fn frame(family: SelectedDesignerFamily) -> SelectedDesignerIdentityFrame {
        let object_type = family.stable_object_type();
        let owner = match family {
            SelectedDesignerFamily::Division => {
                SelectedDesignerOwner::CountryDivisionDesigner
            }
            SelectedDesignerFamily::Equipment | SelectedDesignerFamily::Ship => {
                SelectedDesignerOwner::CountryEquipmentDesignerMode0
            }
            SelectedDesignerFamily::Tank => SelectedDesignerOwner::TankDesignerMode1,
            SelectedDesignerFamily::Plane => SelectedDesignerOwner::PlaneDesignerMode2,
            SelectedDesignerFamily::VariantUpgrade => {
                SelectedDesignerOwner::EquipmentUpgradeDesignerWindow
            }
        };
        SelectedDesignerIdentityFrame {
            frame_hour: 42,
            transition_sequence: 7,
            executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            classifier_manifest_sha256: CLASSIFIER_MANIFEST_SHA256,
            division_click_thunk_artifact_sha256: DIVISION_CLICK_THUNK_ARTIFACT_SHA256,
            division_wrapper_artifact_sha256: DIVISION_WRAPPER_ARTIFACT_SHA256,
            source_record_identity: "a40:selected:42:7".into(),
            owner,
            family,
            expected_ordered_design_count: 3,
            ordered_designs: vec![
                StableDesignIdentity {
                    object_type,
                    stable_id: 9,
                },
                StableDesignIdentity {
                    object_type,
                    stable_id: 3,
                },
                StableDesignIdentity {
                    object_type,
                    stable_id: 41,
                },
            ],
            selected_design: StableDesignIdentity {
                object_type,
                stable_id: 3,
            },
            complete: true,
        }
    }

    #[test]
    fn all_six_families_preserve_pointer_free_stable_selection() {
        for family in [
            SelectedDesignerFamily::Division,
            SelectedDesignerFamily::Equipment,
            SelectedDesignerFamily::Tank,
            SelectedDesignerFamily::Plane,
            SelectedDesignerFamily::Ship,
            SelectedDesignerFamily::VariantUpgrade,
        ] {
            validate_frame(&frame(family)).unwrap();
        }
    }

    #[test]
    fn shared_mode_zero_does_not_collapse_equipment_and_ship() {
        let equipment = frame(SelectedDesignerFamily::Equipment);
        let ship = frame(SelectedDesignerFamily::Ship);
        assert_eq!(equipment.owner, ship.owner);
        assert_ne!(equipment.family, ship.family);
        validate_frame(&equipment).unwrap();
        validate_frame(&ship).unwrap();
    }

    #[test]
    fn wrong_type_owner_order_and_selection_fail_closed() {
        let mut value = frame(SelectedDesignerFamily::Tank);
        value.selected_design = variant(99);
        assert!(validate_frame(&value).is_err());

        let mut value = frame(SelectedDesignerFamily::Division);
        value.ordered_designs[0] = variant(9);
        assert!(validate_frame(&value).is_err());

        let mut value = frame(SelectedDesignerFamily::Plane);
        value.owner = SelectedDesignerOwner::TankDesignerMode1;
        assert!(validate_frame(&value).is_err());

        let mut value = frame(SelectedDesignerFamily::Ship);
        value.expected_ordered_design_count -= 1;
        assert!(validate_frame(&value).is_err());

        let mut value = frame(SelectedDesignerFamily::Equipment);
        value.classifier_manifest_sha256[0] ^= 1;
        assert!(validate_frame(&value).is_err());

        let mut value = frame(SelectedDesignerFamily::Division);
        value.division_click_thunk_artifact_sha256[0] ^= 1;
        assert!(validate_frame(&value).is_err());
    }

    #[test]
    fn d85_transport_roundtrips_exact_order_and_selected_identity() {
        let snapshot = frame(SelectedDesignerFamily::Ship);
        let expected = SelectedDesignerTransportFrame {
            elapsed_wall_ms: 17,
            status_flags: 0,
            frame_hour: snapshot.frame_hour,
            snapshot: Some(snapshot),
        };
        let payload = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&payload).unwrap(), expected);
    }

    #[test]
    fn d85_transport_rejects_torn_or_clean_missing_snapshots() {
        let clean_missing = SelectedDesignerTransportFrame {
            elapsed_wall_ms: 17,
            status_flags: 0,
            frame_hour: 42,
            snapshot: None,
        };
        assert!(encode_payload(&clean_missing).is_err());

        let mut payload = encode_payload(&SelectedDesignerTransportFrame {
            elapsed_wall_ms: 17,
            status_flags: 0,
            frame_hour: 42,
            snapshot: Some(frame(SelectedDesignerFamily::Plane)),
        })
        .unwrap();
        payload.pop();
        assert_eq!(
            decode_payload(&payload),
            Err(SelectedDesignerError::Truncated)
        );
    }

    #[test]
    fn live_source_seam_stays_blocked_until_six_family_fixture_is_bound() {
        struct Source;
        impl ExactSelectedDesignerIdentitySource for Source {
            fn is_exact_a729_selected_designer_source(&self) -> bool {
                true
            }
            fn producer_manifest_sha256(&self) -> [u8; 32] {
                PRODUCER_MANIFEST_SHA256
            }
            fn classifier_manifest_sha256(&self) -> [u8; 32] {
                CLASSIFIER_MANIFEST_SHA256
            }
            fn collect_frame(
                &self,
                _frame_hour: u64,
                family: SelectedDesignerFamily,
            ) -> Result<SelectedDesignerIdentityFrame, String> {
                Ok(frame(family))
            }
        }

        assert!(PROMOTION_AUTHORITY_BOUND);
        assert!(CLASSIFIER_AUTHORITY_BOUND);
        assert!(!LIVE_FIXTURE_AUTHORITY_BOUND);
        assert!(!PRODUCTION_ACTIVATION);
        assert_eq!(
            collect_and_encode(&Source, 42, 17, SelectedDesignerFamily::Tank),
            Err(SelectedDesignerError::ActivationBlocked)
        );
    }
}
