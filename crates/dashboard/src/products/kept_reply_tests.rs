//! Which interactive replies are kept between sessions.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Loads of the fixture products so far.
static LOADS: AtomicUsize = AtomicUsize::new(0);

/// An interactive reply, each `KIND` its own: the second's load notes a fallback.
struct Hover<const KIND: u8>;
impl<const KIND: u8> Product for Hover<KIND> {
    const KEY: &'static str = match KIND {
        0 => "kept-interactive-fixture",
        1 => "kept-fallback-fixture",
        _ => "kept-undrawn-fixture",
    };
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = land_owners::LandOwnersParams;
    type Reply = crate::LandOwners;
    fn load(
        _source: &Source,
        _params: &Self::Params,
        _context: &LoadContext,
        _cancelled: &AtomicBool,
    ) -> std::io::Result<Self::Reply> {
        LOADS.fetch_add(1, Ordering::SeqCst);
        if KIND == 1 {
            installed_content_core::content::credit_overlay_refusals(&std::collections::BTreeSet::from([
                "fixture art".to_owned(),
            ]));
        }
        Ok(crate::LandOwners {
            countries: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

/// An interactive reply is kept once computing it took [`KEEP_INTERACTIVE_AFTER`], and a later
/// load answers it from disk; a quicker one, one whose load noted a fallback, which joins every
/// reply's warnings when first noted, and any once artwork failed to draw, are computed again.
#[test]
fn slow_interactive_replies_are_kept_and_quick_ones_computed_again() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for settings metadata tests");
    let root = std::env::temp_dir().join(format!("kept-interactive-{}", uuid::Uuid::new_v4()));
    let (source, context) = crate::test_fixtures::source_with_content(
        vec![crate::test_fixtures::world_frame(10, false)],
        &root,
    );
    let load = |product: Registered| {
        (product.load)(
            &source,
            "lease",
            serde_json::json!({}),
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        crate::reply_cache::settle();
        LOADS.load(Ordering::SeqCst)
    };
    let quick = load(register::<Hover<0>>());
    assert_eq!(load(register::<Hover<0>>()), quick + 1);
    KEEP_AFTER.set(std::time::Duration::ZERO);
    let kept = load(register::<Hover<0>>());
    assert_eq!(load(register::<Hover<0>>()), kept);
    let fell_back = load(register::<Hover<1>>());
    assert_eq!(load(register::<Hover<1>>()), fell_back + 1);
    DRAW_FAILED.set(true);
    let undrawn = load(register::<Hover<2>>());
    assert_eq!(load(register::<Hover<2>>()), undrawn + 1);
    let _ = std::fs::remove_dir_all(&root);
}
