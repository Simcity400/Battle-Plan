//! A selection that cannot be shown is named on every content reply drawn under it, whichever
//! path served the reply: regressions that read a mount other than the admitted one show here.

use crate::{
    presentation::Presentation,
    products::ContentOptions,
    test_fixtures::{self, source_with_content},
};
use installed_content_core::content::visual::Choice;
use std::sync::atomic::AtomicBool;

/// A recording over a synthetic game with the flag cells the flag reader needs, its next content
/// work admitted under a selection whose only mod is not installed.
fn refused_selection(name: &str) -> (std::path::PathBuf, crate::Source, ContentOptions) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_presentation_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let defines = root.join("common/defines/00_flags.lua");
    std::fs::create_dir_all(defines.parent().unwrap()).unwrap();
    std::fs::write(
        defines,
        "NDefines = { NGraphics = { COUNTRY_FLAG_TEX_WIDTH = 82, COUNTRY_FLAG_TEX_HEIGHT = 52, \
         COUNTRY_FLAG_TEX_MAX_SIZE = 2048, COUNTRY_FLAG_MEDIUM_TEX_WIDTH = 41, \
         COUNTRY_FLAG_MEDIUM_TEX_HEIGHT = 26, COUNTRY_FLAG_MEDIUM_TEX_MAX_SIZE = 1024, \
         COUNTRY_FLAG_SMALL_TEX_WIDTH = 10, COUNTRY_FLAG_SMALL_TEX_HEIGHT = 7, \
         COUNTRY_FLAG_SMALL_TEX_MAX_SIZE = 256 } }",
    )
    .unwrap();
    let (source, context) =
        source_with_content(vec![test_fixtures::source(1, 100, Some(1))], &root);
    let options = context.content.unwrap();
    let gone = Presentation::current(
        &options.options,
        source.replay_ref(),
        &[Choice {
            id: "mod/gone.mod".into(),
            name: "Gone".into(),
        }],
        &AtomicBool::new(false),
    )
    .unwrap();
    source.bind(Some((&options.options, options.allow_unverified)), gone);
    let admitted = source
        .admit_resolved(crate::source::Work::Content(std::sync::Arc::new(
            AtomicBool::new(false),
        )))
        .unwrap();
    (root, source, admitted)
}

fn names_refusal(warnings: &[String]) -> bool {
    warnings
        .iter()
        .any(|line| line.contains("Gone") && line.contains("not shown"))
}

/// Full, medium and small flags, each asked twice so the second is answered from the kept flag
/// context, name the refused selection on every reply.
#[test]
fn every_flag_reply_names_a_refused_selection() {
    let (root, source, admitted) = refused_selection("flags");
    let flags = super::find("country-flags").unwrap();
    for size in ["full", "medium", "small"] {
        for _ in 0..2 {
            let reply = (flags.load)(
                &source,
                "lease",
                serde_json::json!({ "branchId": "1", "hour": 100, "tags": ["GER"], "size": size }),
                &super::LoadContext {
                    content: Some(admitted.clone()),
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
            let warnings: Vec<String> =
                serde_json::from_value(reply["data"]["warnings"].clone()).unwrap();
            assert!(names_refusal(&warnings), "{size}: {warnings:?}");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

/// A selection's mount is resolved at each request's entry, never under a product's cache lock:
/// while one request mounts it slowly, a request at the aircraft catalogue's cache runs to the
/// end, and a request waiting for that mount waits at its entry, released when it is cancelled.
/// Product code never mounts: work whose entry did not resolve its mount fails loudly and mounts
/// nothing under the catalogue's cache.
#[test]
fn a_slow_mount_holds_no_cache_lock_and_its_waiter_is_cancellable() {
    use crate::presentation::Cancel;
    use std::sync::{Arc, Barrier, atomic::Ordering};
    let (root, source, admitted) = refused_selection("slow_mount");
    let aircraft = super::find("aircraft-catalogue").unwrap();
    let page = |content: crate::products::ContentOptions| {
        (aircraft.load)(
            &source,
            "lease",
            serde_json::json!({ "scope": "all", "query": "", "first": 0, "count": 10 }),
            &super::LoadContext {
                content: Some(content),
            },
            &AtomicBool::new(false),
        )
    };
    // A new selection, whose mount the first request computes slowly.
    let slow = Presentation::current(
        &admitted.options,
        source.replay_ref(),
        &[Choice {
            id: "mod/slow.mod".into(),
            name: "Slow".into(),
        }],
        &AtomicBool::new(false),
    )
    .unwrap();
    source.bind(Some((&admitted.options, admitted.allow_unverified)), slow);
    let binding = source.binding().unwrap();
    let (computing, finish) = (Barrier::new(2), Barrier::new(2));
    let cancel = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            binding
                .mount
                .get_or_mount(AtomicBool::new(false).flag(), || {
                    computing.wait();
                    finish.wait();
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ))
                })
        });
        computing.wait();
        let waiter = scope.spawn(|| {
            let content = source
                .admit_resolved(crate::source::Work::Content(
                    Arc::clone(&cancel) as Arc<dyn Cancel>
                ))
                .unwrap();
            page(content)
        });
        // The aircraft catalogue's cache is free: a request of another binding runs to the end.
        page(admitted.clone()).unwrap();
        cancel.store(true, Ordering::Release);
        assert!(waiter.join().unwrap().is_err(), "the waiter is released");
        finish.wait();
        assert!(first.join().unwrap().is_err());
    });
    let unresolved = source
        .admit(crate::source::Work::Content(
            Arc::new(AtomicBool::new(false)) as Arc<dyn Cancel>,
        ))
        .unwrap();
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page(unresolved)));
    assert!(
        read.is_err() || read.unwrap().is_err(),
        "a missed entry fails"
    );
    assert!(
        source.binding().unwrap().mount.get().is_none(),
        "nothing mounted under the catalogue's cache"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A prepared answer served again from the query cache names the refused selection as the
/// answer served fresh did: the line comes from the admission, not from the answer.
#[test]
fn a_kept_prepared_answer_names_a_refused_selection() {
    let (root, source, admitted) = refused_selection("prepared");
    let answered = std::collections::BTreeSet::new();
    for _ in 0..2 {
        let lines = crate::prepared::query::served_lines(
            &source,
            Some(&admitted),
            "occupied-states-manifest",
            "occupied-states-manifest",
            &answered,
        );
        assert!(names_refusal(&lines), "{lines:?}");
    }
    let _ = std::fs::remove_dir_all(root);
}
