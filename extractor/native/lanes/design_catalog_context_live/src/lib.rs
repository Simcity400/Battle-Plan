//! A27 exact a729 equipment-designer view context and d78 carrier.
//!
//! The carrier contains only stable numeric design/country identities and raw
//! controller state. Runtime view, row, variant, handler, and country pointers
//! are consumed inside the signature-gated callback and are never serialized.

use std::collections::BTreeSet;

#[cfg(windows)]
pub mod a729_hook;
pub mod selected_designer_identity;

pub const DOMAIN_ID: u16 = 78;
pub const DOMAIN_NAME: &str = "design_catalog_context_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_DESIGNER_VIEW: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"DCX1");
pub const MAX_SELECTED_COUNTRIES: usize = 4_096;
pub const MAX_ORDERED_DESIGNS: usize = 131_072;

/// Whole-function Ghidra manifests for the transition root and complete grid
/// producer. Both hashes are embedded in every admitted payload.
pub const ROOT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"e62da9e71f6de9b3a87d8d6dd2c32a1db0723be678bc1798c7533cdfa4ebab65");
pub const GRID_MANIFEST_SHA256: [u8; 32] =
    hex32(b"e12cbbd037d94c33a81381b49021116dc1ca7af7325af9bbffe101acd5d82904");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn n(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (n(input[i * 2]) << 4) | n(input[i * 2 + 1]);
        i += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignerFamily {
    CountryEquipment,
    Tank,
    Plane,
}

impl DesignerFamily {
    pub fn from_mode(mode: i32) -> Result<Self, CarrierError> {
        match mode {
            0 => Ok(Self::CountryEquipment),
            1 => Ok(Self::Tank),
            2 => Ok(Self::Plane),
            _ => invalid("designer mode outside exact a729 range 0..=2"),
        }
    }

    pub const fn mode(self) -> i32 {
        match self {
            Self::CountryEquipment => 0,
            Self::Tank => 1,
            Self::Plane => 2,
        }
    }

    pub const fn engine_view_name(self) -> &'static str {
        match self {
            Self::CountryEquipment => "countryequipmentdesignerview",
            Self::Tank => "tank_designer_view",
            Self::Plane => "plane_designer_view",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignerViewContext {
    pub transition_sequence: u64,
    pub active: bool,
    pub family: DesignerFamily,
    pub our_filter_enabled: bool,
    pub foreign_filter_enabled: bool,
    pub obsolete_filter_enabled: bool,
    pub current_country_tag_id: u32,
    /// Exact `view+0xF418/+0xF424` vector order. Tag equivalence remains an
    /// engine law; this carrier intentionally does not canonicalize the vector.
    pub selected_country_tag_ids: Vec<u32>,
    /// Exact accepted row-pool order after `0x1417797A0` returns.
    pub ordered_design_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignCatalogContextFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    /// `None` is permitted only with a nonzero failure status and represents
    /// a scheduled producer that has not observed a complete transition. It
    /// is never decoded as measured-empty or inactive.
    pub context: Option<DesignerViewContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub fn validate_context(context: &DesignerViewContext) -> Result<(), CarrierError> {
    if context.transition_sequence == 0 {
        return invalid("transition sequence is zero");
    }
    if context.current_country_tag_id == 0 {
        return invalid("current country tag identity is zero");
    }
    if context.selected_country_tag_ids.len() > MAX_SELECTED_COUNTRIES {
        return invalid("selected-country vector exceeds bound");
    }
    if context.ordered_design_ids.len() > MAX_ORDERED_DESIGNS {
        return invalid("ordered-design vector exceeds bound");
    }
    if context.selected_country_tag_ids.iter().any(|id| *id == 0) {
        return invalid("selected-country vector contains zero identity");
    }
    let mut selected = BTreeSet::new();
    if context
        .selected_country_tag_ids
        .iter()
        .any(|id| !selected.insert(*id))
    {
        return invalid("selected-country vector contains duplicate raw identity");
    }
    let mut designs = BTreeSet::new();
    if context
        .ordered_design_ids
        .iter()
        .any(|id| *id == 0 || !designs.insert(*id))
    {
        return invalid("ordered-design vector contains zero or duplicate identity");
    }
    if !context.active && !context.ordered_design_ids.is_empty() {
        return invalid("inactive designer view contains materialized design rows");
    }
    Ok(())
}

pub fn validate_frame(frame: &DesignCatalogContextFrame) -> Result<(), CarrierError> {
    if frame.completeness_record_identity.trim().is_empty() {
        return invalid("completeness record identity is empty");
    }
    match &frame.context {
        Some(context) => validate_context(context),
        None if frame.status_flags != 0 => Ok(()),
        None => invalid("missing context requires nonzero failure status"),
    }
}

pub fn encode_payload(frame: &DesignCatalogContextFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::with_capacity(
        128 + frame
            .context
            .as_ref()
            .map(|context| {
                context.selected_country_tag_ids.len() + context.ordered_design_ids.len()
            })
            .unwrap_or(0)
            * 4,
    );
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_EXACT_A729_DESIGNER_VIEW);
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&ROOT_MANIFEST_SHA256);
    out.extend_from_slice(&GRID_MANIFEST_SHA256);
    string(&mut out, &frame.completeness_record_identity)?;
    out.push(u8::from(frame.context.is_some()));
    out.extend_from_slice(&[0; 3]);
    let Some(context) = &frame.context else {
        return Ok(out);
    };
    u64v(&mut out, context.transition_sequence);
    out.push(u8::from(context.active));
    out.push(context.family.mode() as u8);
    out.push(u8::from(context.our_filter_enabled));
    out.push(u8::from(context.foreign_filter_enabled));
    out.push(u8::from(context.obsolete_filter_enabled));
    out.extend_from_slice(&[0; 3]);
    u32v(&mut out, context.current_country_tag_id);
    u32v(&mut out, context.selected_country_tag_ids.len() as u32);
    u32v(&mut out, context.ordered_design_ids.len() as u32);
    for id in &context.selected_country_tag_ids {
        u32v(&mut out, *id);
    }
    for id in &context.ordered_design_ids {
        u32v(&mut out, *id);
    }
    Ok(out)
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
    encode_payload(&DesignCatalogContextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity: format!("a27:a729:source_unavailable:{reason}"),
        context: None,
    })
}

pub fn decode_payload(payload: &[u8]) -> Result<DesignCatalogContextFrame, CarrierError> {
    let mut r = Reader { payload, at: 0 };
    if r.u32()? != MAGIC
        || r.u32()? != SCHEMA_VERSION
        || r.u32()? != AUTHORITY_EXACT_A729_DESIGNER_VIEW
    {
        return invalid("DCX1 magic/schema/authority mismatch");
    }
    let status_flags = r.u32()?;
    let elapsed_wall_ms = r.u64()?;
    let frame_hour = r.u64()?;
    if r.bytes(32)? != ROOT_MANIFEST_SHA256 || r.bytes(32)? != GRID_MANIFEST_SHA256 {
        return invalid("DCX1 producer manifest mismatch");
    }
    let completeness_record_identity = r.string()?;
    let has_context = r.bool()?;
    if r.bytes(3)? != [0, 0, 0] {
        return invalid("DCX1 availability reserved bytes are nonzero");
    }
    if !has_context {
        if r.at != payload.len() {
            return Err(CarrierError::TrailingBytes);
        }
        let frame = DesignCatalogContextFrame {
            elapsed_wall_ms,
            status_flags,
            frame_hour,
            completeness_record_identity,
            context: None,
        };
        validate_frame(&frame)?;
        return Ok(frame);
    }
    let transition_sequence = r.u64()?;
    let active = r.bool()?;
    let family = DesignerFamily::from_mode(r.u8()? as i32)?;
    let our_filter_enabled = r.bool()?;
    let foreign_filter_enabled = r.bool()?;
    let obsolete_filter_enabled = r.bool()?;
    if r.bytes(3)? != [0, 0, 0] {
        return invalid("DCX1 reserved bytes are nonzero");
    }
    let current_country_tag_id = r.u32()?;
    let selected_count = r.u32()? as usize;
    let design_count = r.u32()? as usize;
    if selected_count > MAX_SELECTED_COUNTRIES || design_count > MAX_ORDERED_DESIGNS {
        return invalid("DCX1 vector count exceeds bound");
    }
    let mut selected_country_tag_ids = Vec::with_capacity(selected_count);
    for _ in 0..selected_count {
        selected_country_tag_ids.push(r.u32()?);
    }
    let mut ordered_design_ids = Vec::with_capacity(design_count);
    for _ in 0..design_count {
        ordered_design_ids.push(r.u32()?);
    }
    if r.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = DesignCatalogContextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity,
        context: Some(DesignerViewContext {
            transition_sequence,
            active,
            family,
            our_filter_enabled,
            foreign_filter_enabled,
            obsolete_filter_enabled,
            current_country_tag_id,
            selected_country_tag_ids,
            ordered_design_ids,
        }),
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) -> Result<(), CarrierError> {
    let len =
        u32::try_from(value.len()).map_err(|_| CarrierError::Invalid("string too long".into()))?;
    u32v(out, len);
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
        let bytes = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(bytes)
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
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, CarrierError> {
        let len = self.u32()? as usize;
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| CarrierError::Invalid("string is not UTF-8".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(active: bool) -> DesignCatalogContextFrame {
        DesignCatalogContextFrame {
            elapsed_wall_ms: 33,
            status_flags: 0,
            frame_hour: 99,
            completeness_record_identity: "a27:transition:7".into(),
            context: Some(DesignerViewContext {
                transition_sequence: 7,
                active,
                family: DesignerFamily::Tank,
                our_filter_enabled: true,
                foreign_filter_enabled: false,
                obsolete_filter_enabled: true,
                current_country_tag_id: 17,
                selected_country_tag_ids: vec![17, 42],
                ordered_design_ids: if active { vec![100, 7, 55] } else { vec![] },
            }),
        }
    }

    #[test]
    fn roundtrip_preserves_raw_vector_and_final_order() {
        let expected = frame(true);
        assert_eq!(
            decode_payload(&encode_payload(&expected).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn inactive_is_explicit_and_empty() {
        let expected = frame(false);
        assert_eq!(
            decode_payload(&encode_payload(&expected).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn malformed_identity_order_and_trailing_bytes_fail_closed() {
        let mut bad = frame(true);
        bad.context.as_mut().unwrap().ordered_design_ids.push(7);
        assert!(encode_payload(&bad).is_err());
        let mut bytes = encode_payload(&frame(true)).unwrap();
        bytes.push(0);
        assert_eq!(decode_payload(&bytes), Err(CarrierError::TrailingBytes));
    }

    #[test]
    fn unavailable_is_not_measured_empty() {
        let unavailable = DesignCatalogContextFrame {
            elapsed_wall_ms: 1,
            status_flags: 1,
            frame_hour: 2,
            completeness_record_identity: "a27:source_unavailable".into(),
            context: None,
        };
        assert_eq!(
            decode_payload(&encode_payload(&unavailable).unwrap()).unwrap(),
            unavailable
        );
    }
}
