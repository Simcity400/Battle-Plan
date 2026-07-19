//! XUI03 exact runtime carrier for dynamic lists, grids, and overlap collections.
//!
//! Authored GUI declarations cannot supply controller population, stable item
//! identity, final ordering, filtering, materialization, or scroll state.  This
//! module accepts only an authenticated exact-a729 trace and validates it before
//! composition.  It never reconstructs those facts from pixels or names.

use crate::compose::Rect;
use crate::{ErrorKind, GameUiError, Result};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionTraceAuthority {
    ExactA729Runtime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionItemTrace {
    pub stable_item_id: String,
    pub source_ordinal: u32,
    pub included: bool,
    pub filter_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionGroupTrace {
    pub group_id: String,
    pub start_index: usize,
    pub item_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedSlotTrace {
    pub slot_id: String,
    pub bound_item_id: String,
    pub row_template_id: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicCollectionTrace {
    pub authority: CollectionTraceAuthority,
    pub collection_instance_id: String,
    pub frame_hour: u64,
    pub source_items: Vec<CollectionItemTrace>,
    pub ordered_item_ids: Vec<String>,
    pub group_boundaries: Vec<CollectionGroupTrace>,
    pub row_template_id: String,
    pub materialized_slots: Vec<MaterializedSlotTrace>,
    pub clip_rect: Rect,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
    pub scroll_min_fixed: i64,
    pub scroll_max_fixed: i64,
    pub scroll_current_fixed: i64,
    pub scroll_target_fixed: i64,
}

impl DynamicCollectionTrace {
    pub fn validate(&self) -> Result<()> {
        nonempty(&self.collection_instance_id, "collection_instance_id")?;
        nonempty(&self.row_template_id, "row_template_id")?;
        positive_rect(self.clip_rect, "clip_rect")?;
        if self.viewport_extent_pixels.0 <= 0 || self.viewport_extent_pixels.1 <= 0 {
            return Err(fail("viewport extent must be positive"));
        }
        if self.content_extent_pixels.0 < 0 || self.content_extent_pixels.1 < 0 {
            return Err(fail("content extent must be nonnegative"));
        }
        if self.scroll_max_fixed < self.scroll_min_fixed
            || self.scroll_current_fixed < self.scroll_min_fixed
            || self.scroll_current_fixed > self.scroll_max_fixed
            || self.scroll_target_fixed < self.scroll_min_fixed
            || self.scroll_target_fixed > self.scroll_max_fixed
        {
            return Err(fail(
                "scroll range/current/target is not ordered and clamped",
            ));
        }

        let mut source_ids = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        let mut included = BTreeSet::new();
        for item in &self.source_items {
            nonempty(&item.stable_item_id, "stable_item_id")?;
            if !source_ids.insert(item.stable_item_id.as_str()) {
                return Err(fail("duplicate stable_item_id"));
            }
            if !ordinals.insert(item.source_ordinal) {
                return Err(fail("duplicate source_ordinal"));
            }
            if item.included {
                if item.filter_reason.is_some() {
                    return Err(fail("included item must not carry filter_reason"));
                }
                included.insert(item.stable_item_id.as_str());
            } else if item
                .filter_reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
            {
                return Err(fail("excluded item requires a nonempty filter_reason"));
            }
        }

        let mut ordered = BTreeSet::new();
        for id in &self.ordered_item_ids {
            nonempty(id, "ordered_item_id")?;
            if !ordered.insert(id.as_str()) {
                return Err(fail("duplicate ordered_item_id"));
            }
            if !included.contains(id.as_str()) {
                return Err(fail("ordered item is absent or filtered out"));
            }
        }
        if ordered != included {
            return Err(fail(
                "ordered_item_ids do not exactly account for included source items",
            ));
        }

        if !self.group_boundaries.is_empty() {
            let mut cursor = 0usize;
            let mut group_ids = BTreeSet::new();
            for group in &self.group_boundaries {
                nonempty(&group.group_id, "group_id")?;
                if !group_ids.insert(group.group_id.as_str()) {
                    return Err(fail("duplicate group_id"));
                }
                if group.item_count == 0 || group.start_index != cursor {
                    return Err(fail("group boundaries must be positive and contiguous"));
                }
                cursor = cursor
                    .checked_add(group.item_count)
                    .ok_or_else(|| fail("group boundary overflow"))?;
            }
            if cursor != self.ordered_item_ids.len() {
                return Err(fail("group boundaries do not cover final order"));
            }
        }

        let mut slots = BTreeSet::new();
        let mut materialized_items = BTreeSet::new();
        for slot in &self.materialized_slots {
            nonempty(&slot.slot_id, "slot_id")?;
            if !slots.insert(slot.slot_id.as_str()) {
                return Err(fail("duplicate materialized slot_id in one frame"));
            }
            if !materialized_items.insert(slot.bound_item_id.as_str()) {
                return Err(fail("item materialized into multiple slots in one frame"));
            }
            if slot.row_template_id != self.row_template_id {
                return Err(fail(
                    "materialized slot template differs from collection template",
                ));
            }
            if !ordered.contains(slot.bound_item_id.as_str()) {
                return Err(fail("materialized slot binds an item outside final order"));
            }
            positive_rect(slot.rect, "materialized slot rect")?;
        }
        if self.ordered_item_ids.is_empty() && !self.materialized_slots.is_empty() {
            return Err(fail("measured-empty collection cannot materialize rows"));
        }
        Ok(())
    }

    pub fn visible_slots(&self) -> Result<Vec<&MaterializedSlotTrace>> {
        self.validate()?;
        Ok(self
            .materialized_slots
            .iter()
            .filter(|slot| intersects(slot.rect, self.clip_rect))
            .collect())
    }
}

impl TryFrom<&replay_core::DynamicCollectionTraceRow> for DynamicCollectionTrace {
    type Error = GameUiError;

    fn try_from(row: &replay_core::DynamicCollectionTraceRow) -> Result<Self> {
        let trace = DynamicCollectionTrace {
            authority: CollectionTraceAuthority::ExactA729Runtime,
            collection_instance_id: row.collection_instance_id.clone(),
            frame_hour: row.frame_hour,
            source_items: row
                .source_items
                .iter()
                .map(|item| CollectionItemTrace {
                    stable_item_id: item.stable_item_id.clone(),
                    source_ordinal: item.source_ordinal,
                    included: item.included,
                    filter_reason: item.filter_reason.clone(),
                })
                .collect(),
            ordered_item_ids: row.ordered_item_ids.clone(),
            group_boundaries: row
                .group_boundaries
                .iter()
                .map(|group| CollectionGroupTrace {
                    group_id: group.group_id.clone(),
                    start_index: group.start_index as usize,
                    item_count: group.item_count as usize,
                })
                .collect(),
            row_template_id: row.row_template_id.clone(),
            materialized_slots: row
                .materialized_slots
                .iter()
                .map(|slot| MaterializedSlotTrace {
                    slot_id: slot.slot_id.clone(),
                    bound_item_id: slot.bound_item_id.clone(),
                    row_template_id: slot.row_template_id.clone(),
                    rect: Rect::new(slot.rect.x, slot.rect.y, slot.rect.width, slot.rect.height),
                })
                .collect(),
            clip_rect: Rect::new(
                row.clip_rect.x,
                row.clip_rect.y,
                row.clip_rect.width,
                row.clip_rect.height,
            ),
            content_extent_pixels: (row.content_extent_pixels[0], row.content_extent_pixels[1]),
            viewport_extent_pixels: (row.viewport_extent_pixels[0], row.viewport_extent_pixels[1]),
            scroll_min_fixed: row.scroll_min_fixed,
            scroll_max_fixed: row.scroll_max_fixed,
            scroll_current_fixed: row.scroll_current_fixed,
            scroll_target_fixed: row.scroll_target_fixed,
        };
        trace.validate()?;
        Ok(trace)
    }
}

/// Convert the exact recorder d71 sidecar into production collection traces.
/// Absent/unavailable/mismatched-hour input rejects; no static or pixel fallback exists.
pub fn from_replay_frame(frame: &replay_core::WorldFrame) -> Result<Vec<DynamicCollectionTrace>> {
    let view = frame.dynamic_collection_trace();
    if view.status != replay_core::ViewStatus::Present {
        return Err(fail(
            "replay dynamic collection trace is absent or unavailable",
        ));
    }
    let frame_hour =
        u64::try_from(frame.hour_index()).map_err(|_| fail("replay hour_index is negative"))?;
    if view.frame_hour != Some(frame_hour) {
        return Err(fail(
            "replay collection frame_hour does not match snapshot hour",
        ));
    }
    let traces = view
        .collections
        .iter()
        .map(DynamicCollectionTrace::try_from)
        .collect::<Result<Vec<_>>>()?;
    let mut ids = BTreeSet::new();
    if traces
        .iter()
        .any(|trace| !ids.insert(trace.collection_instance_id.as_str()))
    {
        return Err(fail("duplicate collection_instance_id in replay frame"));
    }
    Ok(traces)
}

fn intersects(a: Rect, b: Rect) -> bool {
    let ar = i64::from(a.x()) + i64::from(a.w());
    let ab = i64::from(a.y()) + i64::from(a.h());
    let br = i64::from(b.x()) + i64::from(b.w());
    let bb = i64::from(b.y()) + i64::from(b.h());
    i64::from(a.x()) < br && ar > i64::from(b.x()) && i64::from(a.y()) < bb && ab > i64::from(b.y())
}

fn positive_rect(rect: Rect, field: &str) -> Result<()> {
    if rect.w() <= 0 || rect.h() <= 0 {
        return Err(fail(format!("{field} must have positive size")));
    }
    Ok(())
}

fn nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(fail(format!("{field} must be nonempty")));
    }
    Ok(())
}

fn fail(message: impl Into<String>) -> GameUiError {
    GameUiError::new(
        ErrorKind::Other,
        format!("XUI03_FAIL_CLOSED: {}", message.into()),
    )
}
