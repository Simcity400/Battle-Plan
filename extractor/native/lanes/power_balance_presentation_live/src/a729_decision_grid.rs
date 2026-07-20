//! Exact-a729 static materialization contract for the power-balance decision grid.
//!
//! This module freezes the concrete A37 controller-to-A28 grid join without
//! pretending that static evidence is a live d71 producer. Raw pointers exist
//! only in the callback-side snapshot. The validated result uses stable pool
//! identities and domain decision IDs; it is not wire-encoded while activation
//! remains false.

use std::collections::{BTreeMap, BTreeSet};

use crate::a729_hook::BATTLE_PLAN_A729_EXECUTABLE_SHA256;

pub const STATIC_MATERIALIZATION_AUTHORITY_BOUND: bool = true;
pub const LIVE_MATERIALIZATION_ACTIVATED: bool = false;

pub const NAMED_OWNER_SETUP_RVA: u32 = 0x01c4_9be0;
pub const DECISION_REBUILD_RVA: u32 = 0x01c4_aa30;
pub const GRID_INSERT_RVA: u32 = 0x01c4_8550;
pub const GRID_CLEAR_RVA: u32 = 0x01c4_8640;
pub const GRID_FINALIZE_RVA: u32 = 0x022b_2f10;
pub const CONTAINER_SYNC_RVA: u32 = 0x022a_fa30;

pub const VIEW_DECISION_CONTAINER_OFFSET: usize = 0x50;
pub const VIEW_DECISION_GRID_OFFSET: usize = 0x58;
pub const GRID_CLIP_OWNER_OFFSET: usize = 0x140;
pub const GRID_CONTENT_ORIGIN_OFFSET: usize = 0x160;
pub const GRID_CONTENT_EXTENT_OFFSET: usize = 0x168;
pub const GRID_MAX_SLOTS_HORIZONTAL_OFFSET: usize = 0x170;
pub const GRID_MAX_SLOTS_VERTICAL_OFFSET: usize = 0x174;
pub const GRID_LAYOUT_MODE_OFFSET: usize = 0x17c;
pub const GRID_NEXT_CELL_ORIENTATION_OFFSET: usize = 0x180;
pub const GRID_COORDINATE_DATA_OFFSET: usize = 0x188;
pub const GRID_COORDINATE_COUNT_OFFSET: usize = 0x194;
pub const GRID_CHILD_DATA_OFFSET: usize = 0x1a0;
pub const GRID_CHILD_COUNT_OFFSET: usize = 0x1ac;
pub const CONTAINER_VERTICAL_SCROLLBAR_OFFSET: usize = 0x15e0;
pub const CONTAINER_HORIZONTAL_SCROLLBAR_OFFSET: usize = 0x15e8;
pub const EXTENDED_SCROLL_MIN_FIXED_OFFSET: usize = 0x1558;
pub const EXTENDED_SCROLL_MAX_FIXED_OFFSET: usize = 0x1560;
pub const EXTENDED_SCROLL_TARGET_FIXED_OFFSET: usize = 0x15f8;
pub const EXTENDED_SCROLL_CURRENT_FIXED_OFFSET: usize = 0x1600;

pub const OWNER_JOIN_MANIFEST_SHA256: &str =
    "0b3a883260f3fc96865eb2da49feaea6b4966b764dd7f278e9c3b4a7e9ac232e";
pub const NAMED_SETUP_MANIFEST_SHA256: &str =
    "b4f25497f2440e4fd71d2d5c3a88c9735af67dc4a3abc5812b079d7e5611447e";
pub const PRESENTER_TYPE_MANIFEST_SHA256: &str =
    "8ca3ae6a28ed53be330c43f5bf13ba81835b61117e92eca23b867e2dd2ca63b8";
pub const GRID_FINALIZER_MANIFEST_SHA256: &str =
    "56cbf05a5f9eee61cfe15a24fbe858d1502b9f2c8f1da5af1e112ecd0366cc5c";
pub const SCROLL_FIELD_MANIFEST_SHA256: &str =
    "3bc03f4a95bb8b7ea10d583a71508205a9456aea57d2df7de7fdc4fc03081c10";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PresenterKind {
    Decision,
    TargetedDecision,
    TimedDecision,
    OnMapLocator,
}

impl PresenterKind {
    pub const ALL: [Self; 4] = [
        Self::Decision,
        Self::TargetedDecision,
        Self::TimedDecision,
        Self::OnMapLocator,
    ];

    pub const fn pool_offset(self) -> usize {
        match self {
            Self::Decision => 0x5a8,
            Self::TargetedDecision => 0x5e8,
            Self::TimedDecision => 0x5c8,
            Self::OnMapLocator => 0x608,
        }
    }

    pub const fn runtime_type(self) -> &'static str {
        match self {
            Self::Decision => "CDecisionViewDecisionItem",
            Self::TargetedDecision => "CDecisionViewTargetedDecisionItem",
            Self::TimedDecision => "CDecisionViewTimedDecisionItem",
            Self::OnMapLocator => "CDecisionViewOnMapLocatorItem",
        }
    }

    pub const fn row_template_id(self) -> &'static str {
        match self {
            Self::Decision => "countrydecisionview.gui#decision_item",
            Self::TargetedDecision => "countrydecisionview.gui#targeted_decision_item",
            Self::TimedDecision => "countrydecisionview.gui#timed_decision_item",
            Self::OnMapLocator => "countrydecisionview.gui#on_map_decision_locator_item",
        }
    }

    const fn slot_component(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::TargetedDecision => "targeted_decision",
            Self::TimedDecision => "timed_decision",
            Self::OnMapLocator => "on_map_locator",
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
pub struct ScrollStateFixed {
    pub min: i64,
    pub max: i64,
    pub current: i64,
    pub target: i64,
}

/// One active prefix of a reusable controller-owned presenter pool.
/// `stable_decision_ids[i]` is the already-resolved domain identity bound to
/// `presenter_ptrs[i]`; neither pointer is allowed into the validated result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPresenterPoolSnapshot {
    pub kind: PresenterKind,
    pub pool_offset: usize,
    pub allocated_count: u32,
    pub active_count: u32,
    pub presenter_ptrs: Vec<usize>,
    pub stable_decision_ids: Vec<String>,
}

/// Post-return snapshot of `0x141c4aa30`, after grid finalization and owner
/// synchronization. The bridge must read these values from the same callback
/// transition; authored dimensions or draw-call reconstruction are invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDecisionGridSnapshot {
    pub executable_sha256: [u8; 32],
    pub owner_join_manifest_sha256: String,
    pub named_setup_manifest_sha256: String,
    pub presenter_type_manifest_sha256: String,
    pub grid_finalizer_manifest_sha256: String,
    pub scroll_field_manifest_sha256: String,
    pub current_thread_id: u32,
    pub owner_thread_id: u32,
    pub post_rebuild_return: bool,
    pub view_ptr: usize,
    pub decision_container_ptr: usize,
    pub view_decision_container_slot_value: usize,
    pub decision_grid_ptr: usize,
    pub view_decision_grid_slot_value: usize,
    pub grid_clip_owner_ptr: usize,
    pub vertical_scrollbar_ptr: usize,
    pub container_vertical_scrollbar_slot_value: usize,
    pub coordinates: Vec<[i32; 2]>,
    pub materialized_child_ptrs: Vec<usize>,
    pub materialized_child_rects: Vec<RectI32>,
    pub pools: Vec<RawPresenterPoolSnapshot>,
    pub clip_rect: RectI32,
    pub content_origin_pixels: [i32; 2],
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    pub vertical_scroll_fixed: ScrollStateFixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedDecisionSlot {
    pub slot_id: String,
    pub bound_decision_id: String,
    pub runtime_type: &'static str,
    pub row_template_id: &'static str,
    pub grid_coordinate: [i32; 2],
    pub rect: RectI32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionGridMaterialization {
    pub collection_instance_id: &'static str,
    pub ordered_decision_ids: Vec<String>,
    pub slots: Vec<MaterializedDecisionSlot>,
    pub clip_rect: RectI32,
    pub content_origin_pixels: [i32; 2],
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    pub vertical_scroll_fixed: ScrollStateFixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterializationError {
    Inactive,
    AuthorityMismatch,
    ThreadMismatch,
    WrongBoundary,
    OwnerJoinMismatch,
    InvalidGeometry,
    InvalidScroll,
    PoolSetMismatch,
    InvalidPool,
    DuplicatePresenter,
    GridArrayMismatch,
    UnboundGridChild,
    DuplicateDecisionIdentity,
}

/// Validate the exact controller/grid join and erase every process pointer.
/// This does not assert pre-filter d71 source completeness; it authenticates
/// only the finalized materialized set and its owner-resolved geometry.
pub fn validate_static_materialization(
    raw: &RawDecisionGridSnapshot,
) -> Result<DecisionGridMaterialization, MaterializationError> {
    if !STATIC_MATERIALIZATION_AUTHORITY_BOUND
        || raw.executable_sha256 != BATTLE_PLAN_A729_EXECUTABLE_SHA256
        || raw.owner_join_manifest_sha256 != OWNER_JOIN_MANIFEST_SHA256
        || raw.named_setup_manifest_sha256 != NAMED_SETUP_MANIFEST_SHA256
        || raw.presenter_type_manifest_sha256 != PRESENTER_TYPE_MANIFEST_SHA256
        || raw.grid_finalizer_manifest_sha256 != GRID_FINALIZER_MANIFEST_SHA256
        || raw.scroll_field_manifest_sha256 != SCROLL_FIELD_MANIFEST_SHA256
    {
        return Err(MaterializationError::AuthorityMismatch);
    }
    if raw.current_thread_id == 0 || raw.current_thread_id != raw.owner_thread_id {
        return Err(MaterializationError::ThreadMismatch);
    }
    if !raw.post_rebuild_return {
        return Err(MaterializationError::WrongBoundary);
    }
    if raw.view_ptr == 0
        || raw.decision_container_ptr == 0
        || raw.decision_grid_ptr == 0
        || raw.grid_clip_owner_ptr == 0
        || raw.vertical_scrollbar_ptr == 0
        || raw.view_decision_container_slot_value != raw.decision_container_ptr
        || raw.view_decision_grid_slot_value != raw.decision_grid_ptr
        || raw.container_vertical_scrollbar_slot_value != raw.vertical_scrollbar_ptr
    {
        return Err(MaterializationError::OwnerJoinMismatch);
    }
    if !positive_rect(raw.clip_rect)
        || raw.content_extent_pixels.iter().any(|value| *value < 0)
        || raw.viewport_extent_pixels.iter().any(|value| *value <= 0)
        || raw
            .materialized_child_rects
            .iter()
            .any(|rect| !positive_rect(*rect))
    {
        return Err(MaterializationError::InvalidGeometry);
    }
    if !valid_scroll(raw.vertical_scroll_fixed) {
        return Err(MaterializationError::InvalidScroll);
    }
    if raw.coordinates.len() != raw.materialized_child_ptrs.len()
        || raw.coordinates.len() != raw.materialized_child_rects.len()
    {
        return Err(MaterializationError::GridArrayMismatch);
    }

    let kinds: BTreeSet<_> = raw.pools.iter().map(|pool| pool.kind).collect();
    let expected_kinds: BTreeSet<_> = PresenterKind::ALL.iter().copied().collect();
    if raw.pools.len() != PresenterKind::ALL.len()
        || kinds != expected_kinds
    {
        return Err(MaterializationError::PoolSetMismatch);
    }

    let mut presenter_bindings = BTreeMap::new();
    for pool in &raw.pools {
        let active = pool.active_count as usize;
        if pool.pool_offset != pool.kind.pool_offset()
            || pool.active_count > pool.allocated_count
            || pool.presenter_ptrs.len() != active
            || pool.stable_decision_ids.len() != active
        {
            return Err(MaterializationError::InvalidPool);
        }
        for (ordinal, (&presenter_ptr, decision_id)) in pool
            .presenter_ptrs
            .iter()
            .zip(&pool.stable_decision_ids)
            .enumerate()
        {
            if presenter_ptr == 0 || decision_id.trim().is_empty() {
                return Err(MaterializationError::InvalidPool);
            }
            if presenter_bindings
                .insert(presenter_ptr, (pool.kind, ordinal, decision_id.as_str()))
                .is_some()
            {
                return Err(MaterializationError::DuplicatePresenter);
            }
        }
    }

    // Every active presenter is published exactly once. On-map locators are
    // allowed to precede the conventionally sorted suffix because that is the
    // exact 0x141c4aa30 insertion law.
    if presenter_bindings.len() != raw.materialized_child_ptrs.len() {
        return Err(MaterializationError::GridArrayMismatch);
    }
    let mut seen_children = BTreeSet::new();
    let mut seen_decisions = BTreeSet::new();
    let mut slots = Vec::with_capacity(raw.materialized_child_ptrs.len());
    for (index, child_ptr) in raw.materialized_child_ptrs.iter().copied().enumerate() {
        if !seen_children.insert(child_ptr) {
            return Err(MaterializationError::DuplicatePresenter);
        }
        let Some(&(kind, pool_ordinal, decision_id)) = presenter_bindings.get(&child_ptr) else {
            return Err(MaterializationError::UnboundGridChild);
        };
        if !seen_decisions.insert(decision_id) {
            return Err(MaterializationError::DuplicateDecisionIdentity);
        }
        slots.push(MaterializedDecisionSlot {
            slot_id: format!(
                "powerbalanceview/decision_grid/{}/{}",
                kind.slot_component(),
                pool_ordinal
            ),
            bound_decision_id: decision_id.to_owned(),
            runtime_type: kind.runtime_type(),
            row_template_id: kind.row_template_id(),
            grid_coordinate: raw.coordinates[index],
            rect: raw.materialized_child_rects[index],
        });
    }

    Ok(DecisionGridMaterialization {
        collection_instance_id: "interface/powerbalanceview.gui#decision_grid",
        ordered_decision_ids: slots
            .iter()
            .map(|slot| slot.bound_decision_id.clone())
            .collect(),
        slots,
        clip_rect: raw.clip_rect,
        content_origin_pixels: raw.content_origin_pixels,
        content_extent_pixels: raw.content_extent_pixels,
        viewport_extent_pixels: raw.viewport_extent_pixels,
        vertical_scroll_fixed: raw.vertical_scroll_fixed,
    })
}

/// Production entrypoint remains fail-closed until a same-transition live
/// fixture and the heterogeneous-template d71 representation are admitted.
pub fn admit_live_materialization(
    raw: &RawDecisionGridSnapshot,
) -> Result<DecisionGridMaterialization, MaterializationError> {
    if !LIVE_MATERIALIZATION_ACTIVATED {
        return Err(MaterializationError::Inactive);
    }
    validate_static_materialization(raw)
}

fn positive_rect(rect: RectI32) -> bool {
    rect.width > 0
        && rect.height > 0
        && rect.x.checked_add(rect.width).is_some()
        && rect.y.checked_add(rect.height).is_some()
}

fn valid_scroll(scroll: ScrollStateFixed) -> bool {
    scroll.min <= scroll.max
        && (scroll.min..=scroll.max).contains(&scroll.current)
        && (scroll.min..=scroll.max).contains(&scroll.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(kind: PresenterKind, ptr: usize, decision: &str) -> RawPresenterPoolSnapshot {
        RawPresenterPoolSnapshot {
            kind,
            pool_offset: kind.pool_offset(),
            allocated_count: 1,
            active_count: 1,
            presenter_ptrs: vec![ptr],
            stable_decision_ids: vec![decision.into()],
        }
    }

    fn snapshot() -> RawDecisionGridSnapshot {
        RawDecisionGridSnapshot {
            executable_sha256: BATTLE_PLAN_A729_EXECUTABLE_SHA256,
            owner_join_manifest_sha256: OWNER_JOIN_MANIFEST_SHA256.into(),
            named_setup_manifest_sha256: NAMED_SETUP_MANIFEST_SHA256.into(),
            presenter_type_manifest_sha256: PRESENTER_TYPE_MANIFEST_SHA256.into(),
            grid_finalizer_manifest_sha256: GRID_FINALIZER_MANIFEST_SHA256.into(),
            scroll_field_manifest_sha256: SCROLL_FIELD_MANIFEST_SHA256.into(),
            current_thread_id: 7,
            owner_thread_id: 7,
            post_rebuild_return: true,
            view_ptr: 0x1000,
            decision_container_ptr: 0x2000,
            view_decision_container_slot_value: 0x2000,
            decision_grid_ptr: 0x3000,
            view_decision_grid_slot_value: 0x3000,
            grid_clip_owner_ptr: 0x4000,
            vertical_scrollbar_ptr: 0x5000,
            container_vertical_scrollbar_slot_value: 0x5000,
            // Exact producer law: on-map source order first, then the sorted
            // conventional suffix. These values are a mechanical unit fixture,
            // not claimed as a captured game transition.
            coordinates: vec![[0, 0], [0, 1], [0, 2], [0, 3]],
            materialized_child_ptrs: vec![0x40, 0x10, 0x20, 0x30],
            materialized_child_rects: vec![
                RectI32 { x: 0, y: 0, width: 502, height: 40 },
                RectI32 { x: 0, y: 40, width: 502, height: 40 },
                RectI32 { x: 0, y: 80, width: 502, height: 40 },
                RectI32 { x: 0, y: 120, width: 502, height: 40 },
            ],
            pools: vec![
                pool(PresenterKind::Decision, 0x10, "decision:a"),
                pool(PresenterKind::TargetedDecision, 0x20, "decision:b"),
                pool(PresenterKind::TimedDecision, 0x30, "decision:c"),
                pool(PresenterKind::OnMapLocator, 0x40, "decision:d"),
            ],
            clip_rect: RectI32 { x: 0, y: 0, width: 502, height: 200 },
            content_origin_pixels: [0, 0],
            content_extent_pixels: [502, 160],
            viewport_extent_pixels: [502, 200],
            vertical_scroll_fixed: ScrollStateFixed { min: 0, max: 100_000, current: 0, target: 0 },
        }
    }

    #[test]
    fn exact_join_erases_pointers_and_preserves_heterogeneous_templates() {
        let materialized = validate_static_materialization(&snapshot()).unwrap();
        assert_eq!(materialized.ordered_decision_ids[0], "decision:d");
        assert_eq!(materialized.slots[0].runtime_type, "CDecisionViewOnMapLocatorItem");
        assert_eq!(materialized.slots[1].row_template_id, "countrydecisionview.gui#decision_item");
        assert_ne!(materialized.slots[0].row_template_id, materialized.slots[1].row_template_id);
        assert_eq!(materialized.slots[0].slot_id, "powerbalanceview/decision_grid/on_map_locator/0");
    }

    #[test]
    fn foreign_child_stale_pool_and_pre_return_snapshot_fail_closed() {
        let mut foreign = snapshot();
        foreign.materialized_child_ptrs[0] = 0x99;
        assert_eq!(validate_static_materialization(&foreign), Err(MaterializationError::UnboundGridChild));

        let mut stale = snapshot();
        stale.pools[0].active_count = 0;
        assert_eq!(validate_static_materialization(&stale), Err(MaterializationError::InvalidPool));

        let mut early = snapshot();
        early.post_rebuild_return = false;
        assert_eq!(validate_static_materialization(&early), Err(MaterializationError::WrongBoundary));
    }

    #[test]
    fn live_admission_stays_inactive() {
        assert_eq!(admit_live_materialization(&snapshot()), Err(MaterializationError::Inactive));
    }
}
