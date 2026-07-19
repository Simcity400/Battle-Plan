use game_ui::compose::Rect;
use game_ui::dynamic_collection::{
    CollectionGroupTrace, CollectionItemTrace, CollectionTraceAuthority, DynamicCollectionTrace,
    MaterializedSlotTrace,
};

fn exact_trace() -> DynamicCollectionTrace {
    DynamicCollectionTrace {
        authority: CollectionTraceAuthority::ExactA729Runtime,
        collection_instance_id: "interface/countrydecisionview.gui#decision-grid@FRA".into(),
        frame_hour: 1234,
        source_items: vec![
            CollectionItemTrace {
                stable_item_id: "decision:a".into(),
                source_ordinal: 0,
                included: true,
                filter_reason: None,
            },
            CollectionItemTrace {
                stable_item_id: "decision:hidden".into(),
                source_ordinal: 1,
                included: false,
                filter_reason: Some("trigger_false".into()),
            },
            CollectionItemTrace {
                stable_item_id: "decision:b".into(),
                source_ordinal: 2,
                included: true,
                filter_reason: None,
            },
        ],
        ordered_item_ids: vec!["decision:b".into(), "decision:a".into()],
        group_boundaries: vec![CollectionGroupTrace {
            group_id: "available".into(),
            start_index: 0,
            item_count: 2,
        }],
        row_template_id: "decision_entry".into(),
        materialized_slots: vec![
            MaterializedSlotTrace {
                slot_id: "slot-0".into(),
                bound_item_id: "decision:b".into(),
                row_template_id: "decision_entry".into(),
                rect: Rect::new(10, 10, 200, 40),
            },
            MaterializedSlotTrace {
                slot_id: "slot-1".into(),
                bound_item_id: "decision:a".into(),
                row_template_id: "decision_entry".into(),
                rect: Rect::new(10, 60, 200, 40),
            },
        ],
        clip_rect: Rect::new(0, 0, 220, 70),
        content_extent_pixels: (220, 100),
        viewport_extent_pixels: (220, 70),
        scroll_min_fixed: 0,
        scroll_max_fixed: 100_000,
        scroll_current_fixed: 25_000,
        scroll_target_fixed: 50_000,
    }
}

#[test]
fn exact_trace_closes_population_order_filter_group_clip_and_virtual_slots() {
    let trace = exact_trace();
    trace.validate().expect("exact trace");
    let visible = trace.visible_slots().expect("visible materialization");
    assert_eq!(
        visible
            .iter()
            .map(|slot| slot.slot_id.as_str())
            .collect::<Vec<_>>(),
        vec!["slot-0", "slot-1"]
    );
}

#[test]
fn unaccounted_identity_or_bad_filter_fails_closed() {
    let mut trace = exact_trace();
    trace.ordered_item_ids.pop();
    assert!(trace
        .validate()
        .unwrap_err()
        .message
        .contains("XUI03_FAIL_CLOSED"));
    let mut trace = exact_trace();
    trace.source_items[1].filter_reason = None;
    assert!(trace
        .validate()
        .unwrap_err()
        .message
        .contains("XUI03_FAIL_CLOSED"));
}

#[test]
fn included_item_with_filter_reason_fails_closed() {
    let mut trace = exact_trace();
    trace.source_items[0].filter_reason = Some("stale-exclusion".into());
    assert!(trace.validate().is_err());
}

#[test]
fn row_slot_reuse_is_explicit_across_frames() {
    let first = exact_trace();
    let mut second = exact_trace();
    second.frame_hour += 1;
    second.materialized_slots[0].bound_item_id = "decision:a".into();
    second.materialized_slots.remove(1);
    second
        .validate()
        .expect("same slot id may rebind in a later authenticated frame");
    assert_eq!(
        first.materialized_slots[0].slot_id,
        second.materialized_slots[0].slot_id
    );
    assert_ne!(
        first.materialized_slots[0].bound_item_id,
        second.materialized_slots[0].bound_item_id
    );
}

#[test]
fn off_clip_materialization_is_retained_but_not_visible() {
    let mut trace = exact_trace();
    trace.materialized_slots[1].rect = Rect::new(10, 80, 200, 40);
    let visible = trace.visible_slots().unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].bound_item_id, "decision:b");
}

#[test]
fn exact_replay_row_converts_without_losing_controller_fields() {
    let row = replay_core::DynamicCollectionTraceRow {
        collection_instance_id: "interface/test.gui#grid@GER".into(),
        frame_hour: 900,
        source_items: vec![replay_core::DynamicCollectionSourceItem {
            stable_item_id: "item:a".into(),
            source_ordinal: 0,
            included: true,
            filter_reason: None,
        }],
        ordered_item_ids: vec!["item:a".into()],
        group_boundaries: vec![replay_core::DynamicCollectionGroup {
            group_id: "all".into(),
            start_index: 0,
            item_count: 1,
        }],
        row_template_id: "row".into(),
        materialized_slots: vec![replay_core::DynamicCollectionMaterializedSlot {
            slot_id: "slot:0".into(),
            bound_item_id: "item:a".into(),
            row_template_id: "row".into(),
            rect: replay_core::DynamicCollectionRect {
                x: 1,
                y: 2,
                width: 30,
                height: 10,
            },
        }],
        clip_rect: replay_core::DynamicCollectionRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        content_extent_pixels: [100, 10],
        viewport_extent_pixels: [100, 100],
        scroll_min_fixed: 0,
        scroll_max_fixed: 100_000,
        scroll_current_fixed: 0,
        scroll_target_fixed: 0,
    };
    let trace = DynamicCollectionTrace::try_from(&row).expect("exact replay carrier");
    assert_eq!(trace.collection_instance_id, row.collection_instance_id);
    assert_eq!(trace.frame_hour, 900);
    assert_eq!(trace.ordered_item_ids, vec!["item:a"]);
    assert_eq!(trace.materialized_slots[0].rect, Rect::new(1, 2, 30, 10));
}
