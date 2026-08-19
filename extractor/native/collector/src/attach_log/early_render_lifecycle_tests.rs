use super::{
    aggregate_terminal_errors, append_terminal_error, finish_early_render_lifecycle,
    finish_shared_runtime_text_lifecycle, rollback_early_hook_pair,
};
use std::cell::{Cell, RefCell};

#[test]
fn completed_capture_uninstalls_early_render_hooks_once() {
    let mut calls = 0u32;
    finish_early_render_lifecycle(true, || {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn teardown_failure_is_fail_closed() {
    let error =
        finish_early_render_lifecycle(true, || Err("detour still active".into())).unwrap_err();
    assert_eq!(
        error,
        "uninstall early a729 render hooks failed: detour still active"
    );
}

#[test]
fn ordinary_attach_does_not_touch_render_hooks() {
    let mut calls = 0u32;
    finish_early_render_lifecycle(false, || {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 0);
}

#[test]
fn terminal_error_aggregation_preserves_every_failure_and_unload_refusal() {
    let mut hook = Some("D45 final drain failed".to_string());
    append_terminal_error(&mut hook, "D45 uninstall failed");
    let error = aggregate_terminal_errors(
        [
            Some("A19 observer unregister failed".to_string()),
            hook,
            None,
        ],
        true,
    )
    .unwrap();
    assert_eq!(
                error,
                "A19 observer unregister failed; D45 final drain failed; D45 uninstall failed; collector hook teardown marked unsafe; DLL unload refused"
            );
}

#[test]
fn early_a25_rollback_attempts_both_owners_in_dependency_order() {
    let calls = RefCell::new(Vec::new());
    let suffix = rollback_early_hook_pair(
        || {
            calls.borrow_mut().push("a25-post-emit");
            Ok(())
        },
        || {
            calls.borrow_mut().push("render-owner");
            Ok(())
        },
    );
    assert!(suffix.is_empty());
    assert_eq!(*calls.borrow(), ["a25-post-emit", "render-owner"]);
}

#[test]
fn shared_text_teardown_calls_each_owner_once_in_dependency_order() {
    let calls = RefCell::new(Vec::new());
    finish_shared_runtime_text_lifecycle(
        || {
            calls.borrow_mut().push("a39");
            Ok(())
        },
        || {
            calls.borrow_mut().push("a25");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(*calls.borrow(), ["a39", "a25"]);
}

#[test]
fn failed_a39_quiescence_preserves_shared_a25_hook_owner() {
    let runtime_text_calls = Cell::new(0u32);
    let error = finish_shared_runtime_text_lifecycle(
        || Err("callbacks still active".into()),
        || {
            runtime_text_calls.set(runtime_text_calls.get() + 1);
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("A39 character-presentation teardown failed"));
    assert_eq!(runtime_text_calls.get(), 0);
}
