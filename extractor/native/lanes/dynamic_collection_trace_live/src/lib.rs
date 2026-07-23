//! A28/XUI03 exact dynamic-collection controller trace carrier (production d71).
//!
//! This module owns the complete non-address producer contract. It deliberately
//! has no screen/layout fallback. Receipt-bound equipment observations and the
//! exact-a729 family hooks below supply fail-closed same-transition traces.
//! The legacy all-families fixture latch remains closed; production scheduling
//! must drain only the independently bound hook-owned families.

pub mod a729_controller_observation;
pub mod a729_air_mio_controller;
#[cfg(windows)]
pub mod a729_air_selection_hook;
#[cfg(windows)]
pub mod a729_mio_history_hook;
#[cfg(windows)]
pub mod a729_facility_smooth_list_hook;
#[cfg(windows)]
pub mod a729_named_overlap_hook;
#[cfg(windows)]
pub mod a729_technology_badge_grid_hook;
pub mod a729_technology_canvas;

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 71;
pub const DOMAIN_NAME: &str = "dynamic_collection_trace_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_CONTROLLER: u32 = 1;
/// Static root/owner/accepted-order authority only. This older direct-root
/// receipt does not schedule d71; owner-harness admission is pinned separately
/// in `a729_controller_observation`.
pub const A28_STATIC_ROOT_ORDER_AUTHORITY_BOUND: bool = true;
pub const A28_DIRECT_ROOT_JOB_ID: &str =
    "20260720T021840Z-w3-a28-direct-retry-5065e703";
pub const A28_DIRECT_ROOT_MANIFEST_SHA256: &str =
    "8ce0d88387228dc8a67553e6772c253f40792971760e2f2b695a87af2199b841";
/// Ordinary production scheduling is active for the independently proven Air
/// selection and paired MIO-history subfamilies only.
pub const A28_D71_SCHEDULED: bool = true;
pub const A28_PASSIVE_SUBFAMILIES_PRODUCTION_BOUND: bool = true;
/// Legacy all-families fixture promotion. The exact Air and paired MIO-history
/// hook-owned constructors do not open this broad latch.
pub const A28_LIVE_PROMOTION_BOUND: bool = false;
pub const MAGIC: u32 = u32::from_le_bytes(*b"DCT1");
pub const HEADER_LEN: usize = 40;
pub const MAX_COLLECTIONS: usize = 4_096;
pub const MAX_ITEMS_PER_COLLECTION: usize = 131_072;
pub const MAX_STRING_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceItemTrace {
    pub stable_item_id: String,
    pub source_ordinal: u32,
    pub included: bool,
    pub filter_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupTrace {
    pub group_id: String,
    pub start_index: u32,
    pub item_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedSlotTrace {
    pub slot_id: String,
    pub bound_item_id: String,
    pub row_template_id: String,
    pub rect: RectI32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionTrace {
    pub collection_instance_id: String,
    pub frame_hour: u64,
    pub source_items: Vec<SourceItemTrace>,
    pub ordered_item_ids: Vec<String>,
    pub group_boundaries: Vec<GroupTrace>,
    /// Controller-level default/family template identity. Individual
    /// materialized slots retain their exact template and may differ for
    /// heterogeneous grids such as `decision_grid`.
    pub row_template_id: String,
    pub materialized_slots: Vec<MaterializedSlotTrace>,
    pub clip_rect: RectI32,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
    pub scroll_min_fixed: i64,
    pub scroll_max_fixed: i64,
    pub scroll_current_fixed: i64,
    pub scroll_target_fixed: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub collections: Vec<CollectionTrace>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub(crate) mod source_seal {
    pub trait Sealed {}
}

pub trait ExactControllerTraceSource: source_seal::Sealed {
    fn is_exact_a729_controller_source(&self) -> bool;
    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String>;
}

pub fn collect_and_encode<S: ExactControllerTraceSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TraceError> {
    if !source.is_exact_a729_controller_source() {
        return Err(TraceError::UnauthenticatedSource);
    }
    let frame = TraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        collections: source
            .collect_frame(frame_hour)
            .map_err(TraceError::Source)?,
    };
    encode_payload(&frame)
}

/// Typed unavailable DCT1 frame. Callers must supply nonzero failure status;
/// an empty zero-status frame would be a fabricated measured-empty result.
pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TraceError> {
    if status_flags == 0 {
        return invalid("source-unavailable DCT1 requires failure status");
    }
    encode_payload(&TraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        collections: Vec::new(),
    })
}

pub fn validate_frame(frame: &TraceFrame) -> Result<(), TraceError> {
    if frame.collections.len() > MAX_COLLECTIONS {
        return invalid("collection count exceeds bound");
    }
    let mut ids = BTreeSet::new();
    for collection in &frame.collections {
        if collection.frame_hour != frame.frame_hour {
            return invalid("collection frame_hour differs from frame header");
        }
        validate_nonempty(&collection.collection_instance_id, "collection_instance_id")?;
        if !ids.insert(collection.collection_instance_id.as_str()) {
            return invalid("duplicate collection_instance_id in frame");
        }
        validate_nonempty(&collection.row_template_id, "row_template_id")?;
        validate_rect(collection.clip_rect, "clip_rect")?;
        if collection.content_extent_pixels.0 < 0 || collection.content_extent_pixels.1 < 0 {
            return invalid("content extent must be nonnegative");
        }
        if collection.viewport_extent_pixels.0 <= 0 || collection.viewport_extent_pixels.1 <= 0 {
            return invalid("viewport extent must be positive");
        }
        if collection.scroll_max_fixed < collection.scroll_min_fixed
            || !(collection.scroll_min_fixed..=collection.scroll_max_fixed)
                .contains(&collection.scroll_current_fixed)
            || !(collection.scroll_min_fixed..=collection.scroll_max_fixed)
                .contains(&collection.scroll_target_fixed)
        {
            return invalid("scroll range/current/target invalid");
        }
        if collection.source_items.len() > MAX_ITEMS_PER_COLLECTION
            || collection.ordered_item_ids.len() > MAX_ITEMS_PER_COLLECTION
            || collection.materialized_slots.len() > MAX_ITEMS_PER_COLLECTION
        {
            return invalid("collection item count exceeds bound");
        }
        let mut source_ids = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        let mut included = BTreeSet::new();
        for item in &collection.source_items {
            validate_nonempty(&item.stable_item_id, "stable_item_id")?;
            if !source_ids.insert(item.stable_item_id.as_str()) {
                return invalid("duplicate stable_item_id");
            }
            if !ordinals.insert(item.source_ordinal) {
                return invalid("duplicate source_ordinal");
            }
            if item.included {
                if item.filter_reason.is_some() {
                    return invalid("included item must not carry filter_reason");
                }
                included.insert(item.stable_item_id.as_str());
            } else {
                validate_nonempty(
                    item.filter_reason.as_deref().unwrap_or(""),
                    "excluded filter_reason",
                )?;
            }
        }
        let mut ordered = BTreeSet::new();
        for item_id in &collection.ordered_item_ids {
            validate_nonempty(item_id, "ordered_item_id")?;
            if !ordered.insert(item_id.as_str()) || !included.contains(item_id.as_str()) {
                return invalid("ordered item is duplicate, absent, or excluded");
            }
        }
        if ordered != included {
            return invalid("ordered ids do not exactly equal included source ids");
        }
        if !collection.group_boundaries.is_empty() {
            let mut cursor = 0usize;
            let mut group_ids = BTreeSet::new();
            for group in &collection.group_boundaries {
                validate_nonempty(&group.group_id, "group_id")?;
                if !group_ids.insert(group.group_id.as_str())
                    || group.item_count == 0
                    || group.start_index as usize != cursor
                {
                    return invalid("group boundaries invalid");
                }
                cursor = cursor
                    .checked_add(group.item_count as usize)
                    .ok_or_else(|| TraceError::Invalid("group boundary overflow".into()))?;
            }
            if cursor != collection.ordered_item_ids.len() {
                return invalid("groups do not cover final order");
            }
        }
        let mut slot_ids = BTreeSet::new();
        let mut bound_ids = BTreeSet::new();
        for slot in &collection.materialized_slots {
            validate_nonempty(&slot.slot_id, "slot_id")?;
            validate_nonempty(&slot.bound_item_id, "bound_item_id")?;
            validate_nonempty(&slot.row_template_id, "materialized row_template_id")?;
            if !slot_ids.insert(slot.slot_id.as_str())
                || !bound_ids.insert(slot.bound_item_id.as_str())
                || !ordered.contains(slot.bound_item_id.as_str())
            {
                return invalid("materialized slot identity/binding invalid");
            }
            validate_rect(slot.rect, "materialized slot rect")?;
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &TraceFrame) -> Result<Vec<u8>, TraceError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_CONTROLLER);
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    put_u32(&mut out, frame.collections.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    for collection in &frame.collections {
        put_string(&mut out, &collection.collection_instance_id)?;
        put_string(&mut out, &collection.row_template_id)?;
        put_u64(&mut out, collection.frame_hour);
        put_rect(&mut out, collection.clip_rect);
        put_i32(&mut out, collection.content_extent_pixels.0);
        put_i32(&mut out, collection.content_extent_pixels.1);
        put_i32(&mut out, collection.viewport_extent_pixels.0);
        put_i32(&mut out, collection.viewport_extent_pixels.1);
        put_i64(&mut out, collection.scroll_min_fixed);
        put_i64(&mut out, collection.scroll_max_fixed);
        put_i64(&mut out, collection.scroll_current_fixed);
        put_i64(&mut out, collection.scroll_target_fixed);
        put_u32(&mut out, collection.source_items.len() as u32);
        put_u32(&mut out, collection.ordered_item_ids.len() as u32);
        put_u32(&mut out, collection.group_boundaries.len() as u32);
        put_u32(&mut out, collection.materialized_slots.len() as u32);
        for item in &collection.source_items {
            put_string(&mut out, &item.stable_item_id)?;
            put_u32(&mut out, item.source_ordinal);
            put_u32(&mut out, u32::from(item.included));
            put_optional_string(&mut out, item.filter_reason.as_deref())?;
        }
        for item_id in &collection.ordered_item_ids {
            put_string(&mut out, item_id)?;
        }
        for group in &collection.group_boundaries {
            put_string(&mut out, &group.group_id)?;
            put_u32(&mut out, group.start_index);
            put_u32(&mut out, group.item_count);
        }
        for slot in &collection.materialized_slots {
            put_string(&mut out, &slot.slot_id)?;
            put_string(&mut out, &slot.bound_item_id)?;
            put_string(&mut out, &slot.row_template_id)?;
            put_rect(&mut out, slot.rect);
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<TraceFrame, TraceError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if reader.u32()? != AUTHORITY_EXACT_A729_CONTROLLER {
        return Err(TraceError::UnauthenticatedSource);
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let count = reader.u32()? as usize;
    if reader.u32()? as usize != HEADER_LEN || count > MAX_COLLECTIONS {
        return invalid("header/count invalid");
    }
    let mut collections = Vec::with_capacity(count);
    for _ in 0..count {
        let collection_instance_id = reader.string()?;
        let row_template_id = reader.string()?;
        let collection_frame_hour = reader.u64()?;
        let clip_rect = reader.rect()?;
        let content_extent_pixels = (reader.i32()?, reader.i32()?);
        let viewport_extent_pixels = (reader.i32()?, reader.i32()?);
        let scroll_min_fixed = reader.i64()?;
        let scroll_max_fixed = reader.i64()?;
        let scroll_current_fixed = reader.i64()?;
        let scroll_target_fixed = reader.i64()?;
        let source_count = bounded_count(reader.u32()?)?;
        let ordered_count = bounded_count(reader.u32()?)?;
        let group_count = bounded_count(reader.u32()?)?;
        let slot_count = bounded_count(reader.u32()?)?;
        let mut source_items = Vec::with_capacity(source_count);
        for _ in 0..source_count {
            source_items.push(SourceItemTrace {
                stable_item_id: reader.string()?,
                source_ordinal: reader.u32()?,
                included: match reader.u32()? {
                    0 => false,
                    1 => true,
                    _ => return invalid("included flag is not boolean"),
                },
                filter_reason: reader.optional_string()?,
            });
        }
        let mut ordered_item_ids = Vec::with_capacity(ordered_count);
        for _ in 0..ordered_count {
            ordered_item_ids.push(reader.string()?);
        }
        let mut group_boundaries = Vec::with_capacity(group_count);
        for _ in 0..group_count {
            group_boundaries.push(GroupTrace {
                group_id: reader.string()?,
                start_index: reader.u32()?,
                item_count: reader.u32()?,
            });
        }
        let mut materialized_slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            materialized_slots.push(MaterializedSlotTrace {
                slot_id: reader.string()?,
                bound_item_id: reader.string()?,
                row_template_id: reader.string()?,
                rect: reader.rect()?,
            });
        }
        collections.push(CollectionTrace {
            collection_instance_id,
            frame_hour: collection_frame_hour,
            source_items,
            ordered_item_ids,
            group_boundaries,
            row_template_id,
            materialized_slots,
            clip_rect,
            content_extent_pixels,
            viewport_extent_pixels,
            scroll_min_fixed,
            scroll_max_fixed,
            scroll_current_fixed,
            scroll_target_fixed,
        });
    }
    if !reader.done() {
        return Err(TraceError::TrailingBytes);
    }
    let frame = TraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        collections,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn invalid<T>(message: &str) -> Result<T, TraceError> {
    Err(TraceError::Invalid(message.into()))
}
fn validate_nonempty(value: &str, field: &str) -> Result<(), TraceError> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        invalid(&format!("{field} empty or oversized"))
    } else {
        Ok(())
    }
}
fn validate_rect(rect: RectI32, field: &str) -> Result<(), TraceError> {
    if rect.width <= 0 || rect.height <= 0 {
        invalid(&format!("{field} nonpositive"))
    } else {
        Ok(())
    }
}
fn bounded_count(value: u32) -> Result<usize, TraceError> {
    let value = value as usize;
    if value > MAX_ITEMS_PER_COLLECTION {
        invalid("count exceeds collection bound")
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
fn put_rect(out: &mut Vec<u8>, rect: RectI32) {
    put_i32(out, rect.x);
    put_i32(out, rect.y);
    put_i32(out, rect.width);
    put_i32(out, rect.height);
}
fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), TraceError> {
    validate_nonempty(value, "wire string")?;
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), TraceError> {
    match value {
        None => put_u32(out, u32::MAX),
        Some(v) => {
            validate_nonempty(v, "optional wire string")?;
            put_u32(out, v.len() as u32);
            out.extend_from_slice(v.as_bytes());
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
    fn take(&mut self, n: usize) -> Result<&'a [u8], TraceError> {
        let end = self.offset.checked_add(n).ok_or(TraceError::Truncated)?;
        let v = self
            .bytes
            .get(self.offset..end)
            .ok_or(TraceError::Truncated)?;
        self.offset = end;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, TraceError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, TraceError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, TraceError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, TraceError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, TraceError> {
        let n = self.u32()? as usize;
        if n == 0 || n > MAX_STRING_BYTES {
            return invalid("wire string length invalid");
        };
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| TraceError::Invalid("wire string not UTF-8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, TraceError> {
        let n = self.u32()?;
        if n == u32::MAX {
            return Ok(None);
        };
        let n = n as usize;
        if n == 0 || n > MAX_STRING_BYTES {
            return invalid("optional string length invalid");
        };
        Ok(Some(String::from_utf8(self.take(n)?.to_vec()).map_err(
            |_| TraceError::Invalid("optional string not UTF-8".into()),
        )?))
    }
    fn rect(&mut self) -> Result<RectI32, TraceError> {
        Ok(RectI32 {
            x: self.i32()?,
            y: self.i32()?,
            width: self.i32()?,
            height: self.i32()?,
        })
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exact() -> TraceFrame {
        TraceFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            collections: vec![CollectionTrace {
                collection_instance_id: "interface/test.gui#grid@GER".into(),
                frame_hour: 900,
                source_items: vec![
                    SourceItemTrace {
                        stable_item_id: "item:a".into(),
                        source_ordinal: 0,
                        included: true,
                        filter_reason: None,
                    },
                    SourceItemTrace {
                        stable_item_id: "item:b".into(),
                        source_ordinal: 1,
                        included: false,
                        filter_reason: Some("hidden".into()),
                    },
                ],
                ordered_item_ids: vec!["item:a".into()],
                group_boundaries: vec![GroupTrace {
                    group_id: "all".into(),
                    start_index: 0,
                    item_count: 1,
                }],
                row_template_id: "row".into(),
                materialized_slots: vec![MaterializedSlotTrace {
                    slot_id: "slot:0".into(),
                    bound_item_id: "item:a".into(),
                    row_template_id: "row".into(),
                    rect: RectI32 {
                        x: 1,
                        y: 2,
                        width: 30,
                        height: 10,
                    },
                }],
                clip_rect: RectI32 {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                content_extent_pixels: (100, 10),
                viewport_extent_pixels: (100, 100),
                scroll_min_fixed: 0,
                scroll_max_fixed: 100_000,
                scroll_current_fixed: 0,
                scroll_target_fixed: 0,
            }],
        }
    }
    #[test]
    fn round_trip_preserves_all_fields() {
        let frame = exact();
        let bytes = encode_payload(&frame).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 40);
        assert_eq!(decode_payload(&bytes).unwrap(), frame);
    }
    #[test]
    fn rejects_order_or_filter_drift() {
        let mut frame = exact();
        frame.collections[0].ordered_item_ids.push("item:b".into());
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact();
        frame.collections[0].source_items[1].filter_reason = None;
        assert!(validate_frame(&frame).is_err());
    }
    #[test]
    fn rejects_wrong_frame_or_slot_binding() {
        let mut frame = exact();
        frame.collections[0].frame_hour = 901;
        assert!(validate_frame(&frame).is_err());
        let mut frame = exact();
        frame.collections[0].materialized_slots[0].bound_item_id = "missing".into();
        assert!(validate_frame(&frame).is_err());
    }
    #[test]
    fn heterogeneous_slot_templates_round_trip_without_collapsing_identity() {
        let mut frame = exact();
        frame.collections[0].source_items.push(SourceItemTrace {
            stable_item_id: "item:targeted".into(),
            source_ordinal: 2,
            included: true,
            filter_reason: None,
        });
        frame.collections[0]
            .ordered_item_ids
            .push("item:targeted".into());
        frame.collections[0].group_boundaries[0].item_count = 2;
        frame.collections[0]
            .materialized_slots
            .push(MaterializedSlotTrace {
                slot_id: "slot:1".into(),
                bound_item_id: "item:targeted".into(),
                row_template_id: "targeted_decision_entry".into(),
                rect: RectI32 {
                    x: 1,
                    y: 14,
                    width: 30,
                    height: 10,
                },
            });
        let bytes = encode_payload(&frame).expect("heterogeneous DCT1");
        let decoded = decode_payload(&bytes).expect("heterogeneous DCT1 round trip");
        assert_eq!(
            decoded.collections[0].materialized_slots[1].row_template_id,
            "targeted_decision_entry"
        );

        frame.collections[0].materialized_slots[1]
            .row_template_id
            .clear();
        assert!(validate_frame(&frame).is_err());
    }
    struct Fake(bool);
    impl source_seal::Sealed for Fake {}
    impl ExactControllerTraceSource for Fake {
        fn is_exact_a729_controller_source(&self) -> bool {
            self.0
        }
        fn collect_frame(&self, _: u64) -> Result<Vec<CollectionTrace>, String> {
            Ok(exact().collections)
        }
    }
    #[test]
    fn collector_seam_rejects_unauthenticated_sources() {
        assert_eq!(
            collect_and_encode(&Fake(false), 900, 0, 0),
            Err(TraceError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Fake(true), 900, 0, 0).is_ok());
    }
}
