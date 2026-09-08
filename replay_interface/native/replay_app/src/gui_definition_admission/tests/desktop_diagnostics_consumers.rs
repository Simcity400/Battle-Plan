//! CPU-only consumer tests using the existing admitted XUI12 authored fixture.
//! No HWND, GPU, product process, external input, or real recording is launched.
use super::{xui12_canvas, xui12_fixture};
use crate::desktop_diagnostics::{clipped_point, pointer_json, project_controls, project_tooltip};

#[test]
fn actual_authored_consumer_and_immutable_projection_agree_after_commit() {
    let (mut authored, admission, root) = xui12_fixture();
    let canvas = xui12_canvas(
        &authored,
        &admission,
        "production_military_line_entry",
        40,
        0,
    );
    authored
        .set_production_lines_canvas(&admission, 160, 120, &canvas, 1)
        .unwrap();
    let mut image = game_ui::font::blank_image(160, 120);
    authored
        .compose_open_panels_into(&mut image, &admission, 1)
        .unwrap();
    assert!(
        authored.manager().composed_snapshot().is_err(),
        "candidate is not committed geometry"
    );
    authored.confirm_presented_frame().unwrap();
    let snapshot = authored.manager().composed_snapshot().unwrap();
    assert!(snapshot.committed_frame_generation > 0);
    let mut gaps = Vec::new();
    let controls = project_controls(
        &snapshot,
        [160, 120],
        |x, y| {
            authored
                .manager()
                .pointer_target_at(x, y)
                .map_err(|e| e.to_string())
        },
        &mut gaps,
    );
    assert!(gaps.is_empty(), "{gaps:?}");
    assert!(
        !controls.is_empty(),
        "actual authored fixture must expose reachable controls"
    );
    assert_eq!(
        snapshot,
        authored.manager().composed_snapshot().unwrap(),
        "diagnostics changed input state"
    );
    let control = &controls[0];
    let rect = |field: &str| {
        let r = control[field].as_array().unwrap();
        game_ui::compose::Rect::new(
            r[0].as_i64().unwrap() as i32,
            r[1].as_i64().unwrap() as i32,
            r[2].as_i64().unwrap() as i32,
            r[3].as_i64().unwrap() as i32,
        )
    };
    let (x, y) = clipped_point(rect("client_rect"), rect("clip_rect"), [160, 120]).unwrap();
    let actual = authored
        .pointer_move(&admission, x, y, 1)
        .unwrap()
        .expect("actual admitted pointer consumer");
    let projected = pointer_json(&actual);
    for field in [
        "window_id",
        "element_id",
        "source_file",
        "surface_id",
        "child_path",
        "runtime_instance_token",
        "client_rect",
        "clip_rect",
    ] {
        assert_eq!(control[field], projected[field], "{field}");
    }
    let after_input = authored.manager().composed_snapshot().unwrap();
    assert_eq!(
        after_input.committed_frame_generation,
        snapshot.committed_frame_generation
    );
    assert_eq!(
        after_input.dispatched_input_transition_generation,
        snapshot.dispatched_input_transition_generation,
        "hover is not authenticated action dispatch"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_pending_consumer_has_no_target_and_reject_retains_committed_generation() {
    let (mut authored, admission, root) = xui12_fixture();
    let canvas = xui12_canvas(
        &authored,
        &admission,
        "production_military_line_entry",
        40,
        0,
    );
    authored
        .set_production_lines_canvas(&admission, 160, 120, &canvas, 1)
        .unwrap();
    let mut image = game_ui::font::blank_image(160, 120);
    authored
        .compose_open_panels_into(&mut image, &admission, 1)
        .unwrap();
    authored.confirm_presented_frame().unwrap();
    let before = authored.manager().composed_snapshot().unwrap();
    let collapsed = xui12_canvas(
        &authored,
        &admission,
        "production_military_line_entry_collapsed",
        20,
        0,
    );
    authored
        .set_production_lines_canvas(&admission, 160, 120, &collapsed, 2)
        .unwrap();
    // The real consumer records pending pointer position, but cannot return an
    // admitted owner from desired UI that has not been presented.
    assert!(authored
        .pointer_move(&admission, 1, 1, 2)
        .unwrap()
        .is_none());
    authored
        .compose_open_panels_into(&mut image, &admission, 2)
        .unwrap();
    assert!(authored.manager().composed_snapshot().is_err());
    authored.reject_unpresented_frame().unwrap();
    let rejected = authored.manager().composed_snapshot().unwrap();
    assert_eq!(
        rejected.committed_frame_generation,
        before.committed_frame_generation
    );
    assert_eq!(rejected.stack, before.stack);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn diagnostics_do_not_invent_pointer_ownership_for_blocked_controls() {
    let (mut authored, admission, root) = xui12_fixture();
    let canvas = xui12_canvas(
        &authored,
        &admission,
        "production_military_line_entry",
        40,
        0,
    );
    authored
        .set_production_lines_canvas(&admission, 160, 120, &canvas, 1)
        .unwrap();
    authored
        .compose_open_panels_into(&mut game_ui::font::blank_image(160, 120), &admission, 1)
        .unwrap();
    authored.confirm_presented_frame().unwrap();
    let snapshot = authored.manager().composed_snapshot().unwrap();
    let mut gaps = Vec::new();
    assert!(project_controls(&snapshot, [160, 120], |_, _| Ok(None), &mut gaps).is_empty());
    assert!(project_controls(
        &snapshot,
        [160, 120],
        |_, _| Err("actual owner unavailable".into()),
        &mut gaps
    )
    .is_empty());
    assert!(!gaps.is_empty());
    assert_eq!(authored.manager().composed_snapshot().unwrap(), snapshot);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tooltip_projection_requires_the_actual_committed_owner_and_that_candidates_draw_receipt() {
    let (mut authored, admission, root) = xui12_fixture();
    let canvas = xui12_canvas(
        &authored,
        &admission,
        "production_military_line_entry",
        40,
        0,
    );
    authored
        .set_production_lines_canvas(&admission, 160, 120, &canvas, 1)
        .unwrap();
    let mut image = game_ui::font::blank_image(160, 120);
    authored
        .compose_open_panels_into(&mut image, &admission, 1)
        .unwrap();
    authored.confirm_presented_frame().unwrap();
    let snapshot = authored.manager().composed_snapshot().unwrap();
    let close = snapshot
        .stack
        .iter()
        .flat_map(|s| &s.hit_targets)
        .find(|t| t.authored_name == "close_button")
        .unwrap();
    let (x, y) = clipped_point(close.screen_rect, close.clip_rect, [160, 120]).unwrap();
    authored.pointer_move(&admission, x, y, 1).unwrap();
    let previous = authored
        .authenticated_tooltip_owner_at_cursor()
        .unwrap()
        .unwrap();
    // Compose a second generation through the shipping authored consumer and
    // obtain its real pending owner. The string below is explicitly test-only
    // blend evidence; this CPU test does not assert an actual desktop draw.
    authored
        .set_production_lines_canvas(&admission, 160, 120, &canvas, 2)
        .unwrap();
    authored
        .compose_open_panels_into(&mut image, &admission, 2)
        .unwrap();
    let candidate = authored
        .manager()
        .authenticated_pending_tooltip_owner_at_cursor()
        .unwrap()
        .unwrap();
    assert!(candidate.frame_generation() > previous.frame_generation());
    let pending = (
        candidate.clone(),
        "CPU fixture candidate instance".to_string(),
    );
    let semantic = vec!["XUI05:rows=1:delayed=false:manager-overlay-composited-last".to_string()];
    let mut gaps = Vec::new();
    assert!(project_tooltip(
        Some(&pending),
        None,
        Some(&semantic),
        || panic!("uncommitted geometry"),
        &mut gaps
    )
    .is_null());
    authored.confirm_presented_frame().unwrap();
    let committed = authored
        .authenticated_tooltip_owner_at_cursor()
        .unwrap()
        .unwrap();
    assert_eq!(candidate, committed);
    authored
        .set_authenticated_tooltip_owner(&committed, &pending.1)
        .unwrap();
    let snapshot = authored.manager().composed_snapshot().unwrap();
    let current = || {
        authored
            .authenticated_tooltip_owner_at_cursor()
            .map_err(|e| e.to_string())
    };
    assert!(
        project_tooltip(None, Some(&snapshot), Some(&semantic), current, &mut gaps).is_null(),
        "hover alone is not draw evidence"
    );
    assert!(project_tooltip(
        Some(&pending),
        Some(&snapshot),
        Some(&[]),
        current,
        &mut gaps
    )
    .is_null());
    assert!(gaps
        .iter()
        .any(|s| s == "tooltip_blend_receipt_unavailable"));
    let row = project_tooltip(
        Some(&pending),
        Some(&snapshot),
        Some(&semantic),
        current,
        &mut gaps,
    );
    assert_eq!(row["presented"], true);
    assert_eq!(row["generation"], snapshot.committed_frame_generation);
    assert_eq!(row["source_file"], candidate.source_file());
    assert_eq!(row["surface_id"], candidate.surface_id());
    assert_eq!(row["child_path"], serde_json::json!(candidate.child_path()));
    assert_eq!(
        row["runtime_instance_token"],
        candidate.runtime_instance_token()
    );
    assert_eq!(row["semantic_receipts"], serde_json::json!(&semantic));
    assert_eq!(snapshot, authored.manager().composed_snapshot().unwrap());
    let stale = (previous, pending.1.clone());
    assert!(project_tooltip(
        Some(&stale),
        Some(&snapshot),
        Some(&semantic),
        current,
        &mut gaps
    )
    .is_null());
    let wrong_draw = (candidate, "different candidate instance".to_string());
    assert!(project_tooltip(
        Some(&wrong_draw),
        Some(&snapshot),
        Some(&semantic),
        current,
        &mut gaps
    )
    .is_null());
    assert!(project_tooltip(
        Some(&pending),
        Some(&snapshot),
        Some(&semantic),
        || Err("owner refused".into()),
        &mut gaps
    )
    .is_null());
    assert!(gaps.iter().any(|s| s.contains("owner refused")));
    std::fs::remove_dir_all(root).unwrap();
}
