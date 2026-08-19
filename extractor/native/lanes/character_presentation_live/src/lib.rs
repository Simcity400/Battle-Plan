use std::collections::{BTreeMap, BTreeSet};

#[cfg(windows)]
pub mod a729_hook;

pub const DOMAIN_ID: u16 = 82;
pub const DOMAIN_NAME: &str = "character_presentation_live";
pub const SCHEMA_VERSION: u32 = 4;
pub const AUTHORITY_EXACT_A729_CHARACTER_PRESENTATION: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"CPR1");
pub const MAX_TRAIT_TREES: usize = 8_192;
pub const MAX_TRAIT_NODES: usize = 131_072;
pub const MAX_ACE_PORTRAITS: usize = 65_536;
pub const MAX_TOKEN_BYTES: usize = 1_024;
pub const MAX_MODIFIER_TEXT_BYTES: usize = 1 << 20;
pub const MAX_TOTAL_MODIFIER_TEXT_BYTES: usize = 64 << 20;

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const TRAIT_COLLECTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"837c8b4db6a6cbc03fa5c950dc24c814f5156d6cf0de08f5041bfcccab7d78c9");
pub const TRAIT_STATE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"63ca92c9e5238a0a342e24209417188b7ceeabd7f01a21c45475f58bb3e29e8c");
pub const TRAIT_PROGRESS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"134d8fc1d5056e05023ebd46e33bda913ba3dde6c1f0945d423297d464dae547");
pub const ACE_REFRESH_MANIFEST_SHA256: [u8; 32] =
    hex32(b"4be836cc67b9f59d748a05a73fa4c5d9c46db357d283d25b756c7ad18fcb2674");
pub const ACE_BIND_SORT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"ea545ce9457982590bd42741d1f573a7874594b7c3f836bf290ae8a276f83396");
pub const ACE_MODIFIER_SETTER_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"3b49c9eb735a03fc711f7322b74a5b86ad64b25ffdbcaa73a390b64a2edfd552");
pub const ACE_MODIFIER_FORMATTER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"79fd3a1e1b9eafb09566b2721260936c8430acffcf0ba408cc866c6292ac8f0b");
pub const ACE_SELECTED_CONTEXT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"b593bfc287648ac2bdced54567db5241f2acfe4032f8aed2a77632e29b468c5c");

pub const XUI17_CARRIER_SCHEMA_VERSION: u32 = 4;
pub const XUI17_CARRIER_AUTHORITY: &str = "exact_a729_g8_g11_post_hour";
pub const XUI17_PRODUCER_CONTRACT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"f907073429230425c6d71e1bde7c722d010d63e98cd4eb140eeb875ec5d10c0c");
pub const XUI17_EVIDENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"fbc359fb46a2c7fe2b593ee0af410a034c1e30988d9c73b542df0ce74302d288");
pub const XUI17_G8_POST_HOUR_BOUNDARY_MANIFEST_SHA256: [u8; 32] =
    hex32(b"9959a34c326dddc65cd171010a70e5cff9f646295bd47b37416203cc9689156f");
pub const XUI17_G8_COMPLETE_MEMBERSHIP_MANIFEST_SHA256: [u8; 32] =
    hex32(b"46c5a9c7ad2d0e3e1b2d38bbb954a12302c773d91b95179aeb438f510fee9b79");
pub const XUI17_G11_ACE_SEMANTICS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"63c52460b7142ac8f0346655931c0650c3abf2ebb484de7ddc3a55fd3a8796e2");
pub const XUI17_G8_SUPERSEDING_BOUNDARY_CLAIM_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"77f02b5552b83323565d341df62a9156a5c73e107c463fa902d8a5b809afa625");

pub const XUI17_COMPONENT_STATUS_SOURCE_NOT_CAPTURED: u32 = 1 << 0;

pub const XUI17_G8_TRAIT_UNAVAILABLE_REASON: &str = "XUI17 G8 requires an all-country passive \
     trait-tree census over the complete D18/D47 join, with a trait-definition census, a \
     per-node definition kind, a fixed-point progress scale and blocked-reason text; domain 82 \
     captures only the selected leader's two trait trees and carries none of those leaves";

pub const XUI17_G11_ACE_UNAVAILABLE_REASON: &str = "XUI17 G11 requires an all-country ace \
     presentation joined to the complete D46 country ace order, carrying graphical culture, \
     gender, portrait pool index and the assigned wing's display name and equipment sprite; \
     domain 82 captures only the selected owner's ace portrait sprite and modifier text";

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraitTreeRoot {
    ControllerOffset120,
    ControllerOffset130,
}

impl TraitTreeRoot {
    pub const fn raw_offset(self) -> u16 {
        match self {
            Self::ControllerOffset120 => 0x120,
            Self::ControllerOffset130 => 0x130,
        }
    }

    fn from_raw_offset(value: u16) -> Result<Self, CarrierError> {
        match value {
            0x120 => Ok(Self::ControllerOffset120),
            0x130 => Ok(Self::ControllerOffset130),
            _ => invalid("trait tree root is not exact controller offset 0x120 or 0x130"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraitNodeState {
    Blocked,
    Pickable,
    Picked,
}

impl TraitNodeState {
    pub const fn raw(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Pickable => 1,
            Self::Picked => 2,
        }
    }

    pub fn from_raw(value: u8) -> Result<Self, CarrierError> {
        match value {
            0 => Ok(Self::Blocked),
            1 => Ok(Self::Pickable),
            2 => Ok(Self::Picked),
            _ => invalid("trait node state is outside exact a729 range 0..=2"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitNodeObservation {
    pub trait_token: String,
    pub state: TraitNodeState,
    pub progress_raw: Option<i64>,
    pub definition_kind: u32,
    pub reason_context_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitTreeObservation {
    pub owner_tag_db_id: u32,
    pub unit_leader_id: u64,
    pub root: TraitTreeRoot,
    pub nodes: Vec<TraitNodeObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcePortraitObservation {
    pub owner_tag_db_id: u32,
    pub ace_id: u64,
    pub visible_ordinal: u32,
    pub portrait_gfx: String,
    pub modifier_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AceListContext {
    SelectedWing { owner_tag_db_id: u32, wing_id: u32 },
    PlaneTypeMask { mask_raw: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPresentationSnapshot {
    pub source_sequence: u64,
    pub trait_trees: Vec<TraitTreeObservation>,
    pub ace_context: AceListContext,
    pub ace_portraits: Vec<AcePortraitObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPresentationFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    pub snapshot: Option<CharacterPresentationSnapshot>,
}

pub trait ExactCharacterPresentationSource {
    fn is_exact_a729_character_presentation_source(&self) -> bool;
    fn read_character_presentation(&self) -> Result<CharacterPresentationSnapshot, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub fn collect_and_encode<S: ExactCharacterPresentationSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    completeness_record_identity: impl Into<String>,
) -> Result<Vec<u8>, CarrierError> {
    if !source.is_exact_a729_character_presentation_source() {
        return Err(CarrierError::Source(
            "source did not authenticate exact a729 character presentation producers".into(),
        ));
    }
    let snapshot = source
        .read_character_presentation()
        .map_err(CarrierError::Source)?;
    encode_payload(&CharacterPresentationFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        completeness_record_identity: completeness_record_identity.into(),
        snapshot: Some(snapshot),
    })
}

pub fn validate_snapshot(snapshot: &CharacterPresentationSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 {
        return invalid("source sequence is zero");
    }
    if snapshot.trait_trees.len() != 2 {
        return invalid("exact selected-leader snapshot must contain both trait tree roots");
    }
    if snapshot.trait_trees.len() > MAX_TRAIT_TREES
        || snapshot.ace_portraits.len() > MAX_ACE_PORTRAITS
    {
        return invalid("character presentation row count exceeds bound");
    }
    match &snapshot.ace_context {
        AceListContext::SelectedWing {
            owner_tag_db_id,
            wing_id,
        } if *owner_tag_db_id != 0 && *wing_id != 0 => {}
        AceListContext::PlaneTypeMask { mask_raw } if *mask_raw != 0 => {}
        _ => return invalid("ace list context contains a zero identity or mask"),
    }
    let total_nodes = snapshot
        .trait_trees
        .iter()
        .try_fold(0usize, |total, tree| total.checked_add(tree.nodes.len()))
        .ok_or_else(|| CarrierError::Invalid("trait node count overflow".into()))?;
    if total_nodes > MAX_TRAIT_NODES {
        return invalid("trait node count exceeds bound");
    }
    let mut tree_keys = BTreeSet::new();
    let mut reason_context_ids = BTreeSet::new();
    let expected_trait_identity = (
        snapshot.trait_trees[0].owner_tag_db_id,
        snapshot.trait_trees[0].unit_leader_id,
    );
    let mut observed_roots = BTreeSet::new();
    for tree in &snapshot.trait_trees {
        if tree.owner_tag_db_id == 0 || tree.unit_leader_id == 0 {
            return invalid("trait tree contains zero owner or leader identity");
        }
        if (tree.owner_tag_db_id, tree.unit_leader_id) != expected_trait_identity {
            return invalid("trait tree roots do not describe one selected leader");
        }
        observed_roots.insert(tree.root);
        if !tree_keys.insert((tree.owner_tag_db_id, tree.unit_leader_id, tree.root)) {
            return invalid("duplicate trait tree identity");
        }
        let mut tokens = BTreeSet::new();
        for node in &tree.nodes {
            validate_token(&node.trait_token, "trait token")?;
            if let Some(context_id) = &node.reason_context_id {
                validate_token(context_id, "trait reason context id")?;
                if !reason_context_ids.insert(context_id.as_str()) {
                    return invalid("trait reason context id is bound to multiple nodes");
                }
            }
            if !tokens.insert(node.trait_token.as_str()) {
                return invalid("duplicate trait token within tree");
            }
            if matches!(node.definition_kind, 2 | 3) != node.progress_raw.is_some() {
                return invalid("trait definition kind contradicts its progress applicability");
            }
        }
    }
    if observed_roots
        != BTreeSet::from([
            TraitTreeRoot::ControllerOffset120,
            TraitTreeRoot::ControllerOffset130,
        ])
    {
        return invalid("exact selected-leader snapshot is missing a trait tree root");
    }
    let mut ace_keys = BTreeSet::new();
    let mut ordinals: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut total_modifier_text_bytes = 0usize;
    for ace in &snapshot.ace_portraits {
        if ace.owner_tag_db_id == 0 || ace.ace_id == 0 {
            return invalid("ace portrait contains zero owner or ace identity");
        }
        validate_token(&ace.portrait_gfx, "ace portrait gfx")?;
        if ace.modifier_text.len() > MAX_MODIFIER_TEXT_BYTES {
            return invalid("ace modifier text exceeds byte bound");
        }
        total_modifier_text_bytes = total_modifier_text_bytes
            .checked_add(ace.modifier_text.len())
            .ok_or_else(|| {
                CarrierError::Invalid("ace modifier text byte total overflowed".into())
            })?;
        if total_modifier_text_bytes > MAX_TOTAL_MODIFIER_TEXT_BYTES {
            return invalid("ace modifier text total exceeds frame byte bound");
        }
        if !ace_keys.insert((ace.owner_tag_db_id, ace.ace_id)) {
            return invalid("duplicate owner-qualified ace identity");
        }
        ordinals
            .entry(ace.owner_tag_db_id)
            .or_default()
            .push(ace.visible_ordinal);
    }
    let ace_owners = snapshot
        .ace_portraits
        .iter()
        .map(|ace| ace.owner_tag_db_id)
        .collect::<BTreeSet<_>>();
    if ace_owners.len() > 1 {
        return invalid("ace presentation rows span multiple active owners");
    }
    if let AceListContext::SelectedWing {
        owner_tag_db_id, ..
    } = &snapshot.ace_context
    {
        if ace_owners.iter().any(|owner| owner != owner_tag_db_id) {
            return invalid("selected-wing context owner contradicts ace rows");
        }
    }
    for mut values in ordinals.into_values() {
        values.sort_unstable();
        if values != (0..values.len() as u32).collect::<Vec<_>>() {
            return invalid("ace visible ordinals are not contiguous per owner");
        }
    }
    Ok(())
}

pub fn validate_frame(frame: &CharacterPresentationFrame) -> Result<(), CarrierError> {
    validate_token(
        &frame.completeness_record_identity,
        "completeness record identity",
    )?;
    match &frame.snapshot {
        Some(snapshot) => validate_snapshot(snapshot),
        None if frame.status_flags != 0 => Ok(()),
        None => invalid("missing snapshot requires nonzero failure status"),
    }
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    reason: &str,
) -> Result<Vec<u8>, CarrierError> {
    if status_flags == 0 || reason.trim().is_empty() {
        return invalid("source-unavailable frame requires failure status and reason");
    }
    encode_payload(&CharacterPresentationFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity: format!("xui17:a729:source_unavailable:{reason}"),
        snapshot: None,
    })
}

pub fn encode_payload(frame: &CharacterPresentationFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_EXACT_A729_CHARACTER_PRESENTATION);
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    for hash in [
        EXECUTABLE_SHA256,
        TRAIT_COLLECTION_MANIFEST_SHA256,
        TRAIT_STATE_MANIFEST_SHA256,
        TRAIT_PROGRESS_MANIFEST_SHA256,
        ACE_REFRESH_MANIFEST_SHA256,
        ACE_BIND_SORT_MANIFEST_SHA256,
        ACE_MODIFIER_SETTER_ARTIFACT_SHA256,
        ACE_MODIFIER_FORMATTER_MANIFEST_SHA256,
        ACE_SELECTED_CONTEXT_MANIFEST_SHA256,
    ] {
        out.extend_from_slice(&hash);
    }
    string(&mut out, &frame.completeness_record_identity)?;
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 3]);
    let Some(snapshot) = &frame.snapshot else {
        return Ok(out);
    };
    u64v(&mut out, snapshot.source_sequence);
    match &snapshot.ace_context {
        AceListContext::SelectedWing {
            owner_tag_db_id,
            wing_id,
        } => {
            out.push(1);
            out.extend_from_slice(&[0; 3]);
            u32v(&mut out, *owner_tag_db_id);
            u32v(&mut out, *wing_id);
        }
        AceListContext::PlaneTypeMask { mask_raw } => {
            out.push(2);
            out.extend_from_slice(&[0; 3]);
            u32v(&mut out, *mask_raw);
            u32v(&mut out, 0);
        }
    }
    u32v(&mut out, snapshot.trait_trees.len() as u32);
    u32v(&mut out, snapshot.ace_portraits.len() as u32);
    for tree in &snapshot.trait_trees {
        u32v(&mut out, tree.owner_tag_db_id);
        u64v(&mut out, tree.unit_leader_id);
        u16v(&mut out, tree.root.raw_offset());
        u16v(&mut out, 0);
        u32v(&mut out, tree.nodes.len() as u32);
        for node in &tree.nodes {
            out.push(node.state.raw());
            out.push(u8::from(node.progress_raw.is_some()));
            out.push(u8::from(node.reason_context_id.is_some()));
            out.push(0);
            i64v(&mut out, node.progress_raw.unwrap_or_default());
            u32v(&mut out, node.definition_kind);
            string(&mut out, &node.trait_token)?;
            if let Some(context_id) = &node.reason_context_id {
                string(&mut out, context_id)?;
            }
        }
    }
    for ace in &snapshot.ace_portraits {
        u32v(&mut out, ace.owner_tag_db_id);
        u64v(&mut out, ace.ace_id);
        u32v(&mut out, ace.visible_ordinal);
        string(&mut out, &ace.portrait_gfx)?;
        text(&mut out, &ace.modifier_text)?;
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<CharacterPresentationFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC
        || reader.u32()? != SCHEMA_VERSION
        || reader.u32()? != AUTHORITY_EXACT_A729_CHARACTER_PRESENTATION
    {
        return invalid("CPR1 magic/schema/authority mismatch");
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    for expected in [
        EXECUTABLE_SHA256,
        TRAIT_COLLECTION_MANIFEST_SHA256,
        TRAIT_STATE_MANIFEST_SHA256,
        TRAIT_PROGRESS_MANIFEST_SHA256,
        ACE_REFRESH_MANIFEST_SHA256,
        ACE_BIND_SORT_MANIFEST_SHA256,
        ACE_MODIFIER_SETTER_ARTIFACT_SHA256,
        ACE_MODIFIER_FORMATTER_MANIFEST_SHA256,
        ACE_SELECTED_CONTEXT_MANIFEST_SHA256,
    ] {
        if reader.bytes(32)? != expected {
            return invalid("CPR1 executable or producer manifest mismatch");
        }
    }
    let completeness_record_identity = reader.string()?;
    let has_snapshot = reader.bool()?;
    if reader.bytes(3)? != [0, 0, 0] {
        return invalid("CPR1 availability reserved bytes are nonzero");
    }
    let snapshot = if has_snapshot {
        let source_sequence = reader.u64()?;
        let context_kind = reader.u8()?;
        if reader.bytes(3)? != [0, 0, 0] {
            return invalid("CPR1 ace context reserved bytes are nonzero");
        }
        let context_a = reader.u32()?;
        let context_b = reader.u32()?;
        let ace_context = match (context_kind, context_a, context_b) {
            (1, owner_tag_db_id, wing_id) if owner_tag_db_id != 0 && wing_id != 0 => {
                AceListContext::SelectedWing {
                    owner_tag_db_id,
                    wing_id,
                }
            }
            (2, mask_raw, 0) if mask_raw != 0 => AceListContext::PlaneTypeMask { mask_raw },
            _ => return invalid("CPR1 ace context is invalid"),
        };
        let tree_count = reader.u32()? as usize;
        let ace_count = reader.u32()? as usize;
        if tree_count > MAX_TRAIT_TREES || ace_count > MAX_ACE_PORTRAITS {
            return invalid("CPR1 row count exceeds bound");
        }
        let mut trait_trees = Vec::with_capacity(tree_count);
        let mut total_nodes = 0usize;
        for _ in 0..tree_count {
            let owner_tag_db_id = reader.u32()?;
            let unit_leader_id = reader.u64()?;
            let root = TraitTreeRoot::from_raw_offset(reader.u16()?)?;
            if reader.u16()? != 0 {
                return invalid("CPR1 trait tree reserved field is nonzero");
            }
            let node_count = reader.u32()? as usize;
            total_nodes = total_nodes
                .checked_add(node_count)
                .ok_or_else(|| CarrierError::Invalid("trait node count overflow".into()))?;
            if total_nodes > MAX_TRAIT_NODES {
                return invalid("CPR1 trait node count exceeds bound");
            }
            let mut nodes = Vec::with_capacity(node_count);
            for _ in 0..node_count {
                let state = TraitNodeState::from_raw(reader.u8()?)?;
                let has_progress = reader.bool()?;
                let has_reason_context = reader.bool()?;
                if reader.u8()? != 0 {
                    return invalid("CPR1 trait node reserved byte is nonzero");
                }
                let progress = reader.i64()?;
                if !has_progress && progress != 0 {
                    return invalid("CPR1 absent trait progress has nonzero storage");
                }
                let definition_kind = reader.u32()?;
                let trait_token = reader.string()?;
                let reason_context_id = if has_reason_context {
                    Some(reader.string()?)
                } else {
                    None
                };
                nodes.push(TraitNodeObservation {
                    trait_token,
                    state,
                    progress_raw: has_progress.then_some(progress),
                    definition_kind,
                    reason_context_id,
                });
            }
            trait_trees.push(TraitTreeObservation {
                owner_tag_db_id,
                unit_leader_id,
                root,
                nodes,
            });
        }
        let mut ace_portraits = Vec::with_capacity(ace_count);
        for _ in 0..ace_count {
            ace_portraits.push(AcePortraitObservation {
                owner_tag_db_id: reader.u32()?,
                ace_id: reader.u64()?,
                visible_ordinal: reader.u32()?,
                portrait_gfx: reader.string()?,
                modifier_text: reader.text()?,
            });
        }
        Some(CharacterPresentationSnapshot {
            source_sequence,
            trait_trees,
            ace_context,
            ace_portraits,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = CharacterPresentationFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn validate_token(value: &str, field: &str) -> Result<(), CarrierError> {
    if value.trim().is_empty() || value.len() > MAX_TOKEN_BYTES {
        invalid(format!("{field} is empty or exceeds byte bound"))
    } else {
        Ok(())
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u16v(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i64v(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) -> Result<(), CarrierError> {
    validate_token(value, "serialized string")?;
    u32v(
        out,
        u32::try_from(value.len())
            .map_err(|_| CarrierError::Invalid("serialized string is too long".into()))?,
    );
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn text(out: &mut Vec<u8>, value: &str) -> Result<(), CarrierError> {
    if value.len() > MAX_MODIFIER_TEXT_BYTES {
        return invalid("serialized text exceeds byte bound");
    }
    u32v(
        out,
        u32::try_from(value.len())
            .map_err(|_| CarrierError::Invalid("serialized text is too long".into()))?,
    );
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, CarrierError> {
        Ok(self.bytes(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("invalid bool byte"),
        }
    }
    fn u16(&mut self) -> Result<u16, CarrierError> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, CarrierError> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, CarrierError> {
        let len = self.u32()? as usize;
        if len > MAX_TOKEN_BYTES {
            return invalid("CPR1 string exceeds byte bound");
        }
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| CarrierError::Invalid("CPR1 string is not UTF-8".into()))
    }
    fn text(&mut self) -> Result<String, CarrierError> {
        let len = self.u32()? as usize;
        if len > MAX_MODIFIER_TEXT_BYTES {
            return invalid("CPR1 text exceeds byte bound");
        }
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| CarrierError::Invalid("CPR1 text is not UTF-8".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> CharacterPresentationFrame {
        CharacterPresentationFrame {
            elapsed_wall_ms: 12,
            status_flags: 0,
            frame_hour: 34,
            completeness_record_identity: "xui17:a729:refresh:9".into(),
            snapshot: Some(CharacterPresentationSnapshot {
                source_sequence: 9,
                ace_context: AceListContext::PlaneTypeMask { mask_raw: 0x400 },
                trait_trees: vec![
                    TraitTreeObservation {
                        owner_tag_db_id: 1,
                        unit_leader_id: 7,
                        root: TraitTreeRoot::ControllerOffset120,
                        nodes: vec![
                            TraitNodeObservation {
                                trait_token: "panzer_leader".into(),
                                state: TraitNodeState::Picked,
                                progress_raw: None,
                                definition_kind: 4,
                                reason_context_id: Some("leader_traits::trait_tooltip::0".into()),
                            },
                            TraitNodeObservation {
                                trait_token: "armor_officer".into(),
                                state: TraitNodeState::Pickable,
                                progress_raw: Some(42),
                                definition_kind: 2,
                                reason_context_id: None,
                            },
                        ],
                    },
                    TraitTreeObservation {
                        owner_tag_db_id: 1,
                        unit_leader_id: 7,
                        root: TraitTreeRoot::ControllerOffset130,
                        nodes: Vec::new(),
                    },
                ],
                ace_portraits: vec![AcePortraitObservation {
                    owner_tag_db_id: 1,
                    ace_id: 11,
                    visible_ordinal: 0,
                    portrait_gfx: "GFX_GER_ace_m_7".into(),
                    modifier_text: "+10% agility\n+5% air attack".into(),
                }],
            }),
        }
    }

    #[test]
    fn exact_observations_roundtrip_progress_and_a25_reason_context_distinctly() {
        let expected = frame();
        let encoded = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), expected);
        assert_eq!(
            decode_payload(&encoded)
                .unwrap()
                .snapshot
                .unwrap()
                .trait_trees[0]
                .nodes[0]
                .progress_raw,
            None
        );
        assert_eq!(
            decode_payload(&encoded)
                .unwrap()
                .snapshot
                .unwrap()
                .trait_trees[0]
                .nodes[1]
                .progress_raw,
            Some(42)
        );
        assert_eq!(
            decode_payload(&encoded)
                .unwrap()
                .snapshot
                .unwrap()
                .trait_trees[0]
                .nodes[0]
                .reason_context_id
                .as_deref(),
            Some("leader_traits::trait_tooltip::0")
        );
    }

    #[test]
    fn measured_empty_modifier_text_roundtrips_distinct_from_unavailable() {
        let mut expected = frame();
        expected.snapshot.as_mut().unwrap().ace_portraits[0]
            .modifier_text
            .clear();
        let encoded = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), expected);
    }

    #[test]
    fn oversized_modifier_text_fails_closed() {
        let mut oversized = frame();
        oversized.snapshot.as_mut().unwrap().ace_portraits[0].modifier_text =
            "x".repeat(MAX_MODIFIER_TEXT_BYTES + 1);
        assert!(encode_payload(&oversized).is_err());
    }

    #[test]
    fn state_range_identity_and_owner_ordinals_fail_closed() {
        assert!(TraitNodeState::from_raw(3).is_err());
        let mut duplicate = frame();
        duplicate
            .snapshot
            .as_mut()
            .unwrap()
            .ace_portraits
            .push(AcePortraitObservation {
                owner_tag_db_id: 1,
                ace_id: 12,
                visible_ordinal: 2,
                portrait_gfx: "GFX_GER_ace_m_8".into(),
                modifier_text: String::new(),
            });
        assert!(encode_payload(&duplicate).is_err());
    }

    #[test]
    fn unavailable_is_distinct_from_measured_empty() {
        assert!(encode_source_unavailable(1, 2, 0, "missing hook").is_err());
        let bytes = encode_source_unavailable(1, 2, 1, "missing hook").unwrap();
        assert!(decode_payload(&bytes).unwrap().snapshot.is_none());
    }

    #[test]
    fn one_a25_reason_context_cannot_bind_multiple_trait_nodes() {
        let mut duplicate = frame();
        let repeated = duplicate.snapshot.as_ref().unwrap().trait_trees[0].nodes[0].clone();
        duplicate.snapshot.as_mut().unwrap().trait_trees[1]
            .nodes
            .push(repeated);
        assert!(encode_payload(&duplicate).is_err());
    }

    #[test]
    fn exact_selected_leader_requires_both_roots_with_one_identity() {
        let mut missing_root = frame();
        missing_root.snapshot.as_mut().unwrap().trait_trees.pop();
        assert!(encode_payload(&missing_root).is_err());

        let mut mixed_leaders = frame();
        mixed_leaders.snapshot.as_mut().unwrap().trait_trees[1].unit_leader_id = 8;
        assert!(encode_payload(&mixed_leaders).is_err());
    }
}
