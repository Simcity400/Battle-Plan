//! XUI17 exact a729 trait-tree state/order and ace portrait observations.
//!
//! This pointer-free carrier records only values produced by authenticated
//! engine routes. Trait progress deliberately remains optional: the retained
//! evidence proves node membership, source order, and state, but not a complete
//! progress producer. An absent progress value must never be rendered as zero.

use std::collections::{BTreeMap, BTreeSet};

#[cfg(windows)]
pub mod a729_hook;

pub const DOMAIN_ID: u16 = 82;
pub const DOMAIN_NAME: &str = "character_presentation_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_CHARACTER_PRESENTATION: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"CPR1");
pub const MAX_TRAIT_TREES: usize = 8_192;
pub const MAX_TRAIT_NODES: usize = 131_072;
pub const MAX_ACE_PORTRAITS: usize = 65_536;
pub const MAX_TOKEN_BYTES: usize = 1_024;

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const TRAIT_COLLECTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"837c8b4db6a6cbc03fa5c950dc24c814f5156d6cf0de08f5041bfcccab7d78c9");
pub const TRAIT_STATE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"63ca92c9e5238a0a342e24209417188b7ceeabd7f01a21c45475f58bb3e29e8c");
pub const ACE_REFRESH_MANIFEST_SHA256: [u8; 32] =
    hex32(b"4be836cc67b9f59d748a05a73fa4c5d9c46db357d283d25b756c7ad18fcb2674");
pub const ACE_BIND_SORT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"ea545ce9457982590bd42741d1f573a7874594b7c3f836bf290ae8a276f83396");

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
    /// Stable game-definition token resolved while the exact definition
    /// pointer retained at CUnitLeaderTraitTreeItem+0x20 is still live.
    pub trait_token: String,
    pub state: TraitNodeState,
    /// Not populated until its independent producer route is authenticated.
    pub progress_raw: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitTreeObservation {
    pub owner_tag_db_id: u32,
    pub unit_leader_id: u64,
    pub root: TraitTreeRoot,
    /// Exact CUnitLeaderTraitTree+0x28/+0x34 vector order.
    pub nodes: Vec<TraitNodeObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcePortraitObservation {
    pub owner_tag_db_id: u32,
    pub ace_id: u64,
    /// Exact post-sort CAcesView row order for this owner.
    pub visible_ordinal: u32,
    /// Final sprite selected by the owner-qualified literal/fallback route.
    pub portrait_gfx: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPresentationSnapshot {
    pub source_sequence: u64,
    pub trait_trees: Vec<TraitTreeObservation>,
    pub ace_portraits: Vec<AcePortraitObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPresentationFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    /// `None` is source-unavailable, never measured-empty, and therefore
    /// requires a nonzero failure status.
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
    if snapshot.trait_trees.len() > MAX_TRAIT_TREES
        || snapshot.ace_portraits.len() > MAX_ACE_PORTRAITS
    {
        return invalid("character presentation row count exceeds bound");
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
    for tree in &snapshot.trait_trees {
        if tree.owner_tag_db_id == 0 || tree.unit_leader_id == 0 {
            return invalid("trait tree contains zero owner or leader identity");
        }
        if !tree_keys.insert((tree.owner_tag_db_id, tree.unit_leader_id, tree.root)) {
            return invalid("duplicate trait tree identity");
        }
        let mut tokens = BTreeSet::new();
        for node in &tree.nodes {
            validate_token(&node.trait_token, "trait token")?;
            if !tokens.insert(node.trait_token.as_str()) {
                return invalid("duplicate trait token within tree");
            }
        }
    }
    let mut ace_keys = BTreeSet::new();
    let mut ordinals: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for ace in &snapshot.ace_portraits {
        if ace.owner_tag_db_id == 0 || ace.ace_id == 0 {
            return invalid("ace portrait contains zero owner or ace identity");
        }
        validate_token(&ace.portrait_gfx, "ace portrait gfx")?;
        if !ace_keys.insert((ace.owner_tag_db_id, ace.ace_id)) {
            return invalid("duplicate owner-qualified ace identity");
        }
        ordinals
            .entry(ace.owner_tag_db_id)
            .or_default()
            .push(ace.visible_ordinal);
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
        ACE_REFRESH_MANIFEST_SHA256,
        ACE_BIND_SORT_MANIFEST_SHA256,
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
            u16v(&mut out, 0);
            i64v(&mut out, node.progress_raw.unwrap_or_default());
            string(&mut out, &node.trait_token)?;
        }
    }
    for ace in &snapshot.ace_portraits {
        u32v(&mut out, ace.owner_tag_db_id);
        u64v(&mut out, ace.ace_id);
        u32v(&mut out, ace.visible_ordinal);
        string(&mut out, &ace.portrait_gfx)?;
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
        ACE_REFRESH_MANIFEST_SHA256,
        ACE_BIND_SORT_MANIFEST_SHA256,
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
                if reader.u16()? != 0 {
                    return invalid("CPR1 trait node reserved field is nonzero");
                }
                let progress = reader.i64()?;
                if !has_progress && progress != 0 {
                    return invalid("CPR1 absent trait progress has nonzero storage");
                }
                nodes.push(TraitNodeObservation {
                    trait_token: reader.string()?,
                    state,
                    progress_raw: has_progress.then_some(progress),
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
            });
        }
        Some(CharacterPresentationSnapshot {
            source_sequence,
            trait_trees,
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
                trait_trees: vec![TraitTreeObservation {
                    owner_tag_db_id: 1,
                    unit_leader_id: 7,
                    root: TraitTreeRoot::ControllerOffset120,
                    nodes: vec![
                        TraitNodeObservation {
                            trait_token: "panzer_leader".into(),
                            state: TraitNodeState::Picked,
                            progress_raw: None,
                        },
                        TraitNodeObservation {
                            trait_token: "armor_officer".into(),
                            state: TraitNodeState::Pickable,
                            progress_raw: Some(42),
                        },
                    ],
                }],
                ace_portraits: vec![AcePortraitObservation {
                    owner_tag_db_id: 1,
                    ace_id: 11,
                    visible_ordinal: 0,
                    portrait_gfx: "GFX_GER_ace_m_7".into(),
                }],
            }),
        }
    }

    #[test]
    fn exact_observations_roundtrip_without_inventing_progress() {
        let expected = frame();
        let encoded = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), expected);
        assert_eq!(
            decode_payload(&encoded).unwrap().snapshot.unwrap().trait_trees[0].nodes[0]
                .progress_raw,
            None
        );
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
            });
        assert!(encode_payload(&duplicate).is_err());
    }

    #[test]
    fn unavailable_is_distinct_from_measured_empty() {
        assert!(encode_source_unavailable(1, 2, 0, "missing hook").is_err());
        let bytes = encode_source_unavailable(1, 2, 1, "missing hook").unwrap();
        assert!(decode_payload(&bytes).unwrap().snapshot.is_none());
    }
}
