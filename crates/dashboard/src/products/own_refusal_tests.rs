//! What a product reply names of the catalogue's text refusals.

use super::*;

/// A reply names the refusals its own load noted, never another consumer's, so it is the same
/// whichever replies ran before it.
#[test]
fn a_reply_names_its_own_refusals_and_never_another_consumers() {
    struct Refusing;
    impl Product for Refusing {
        const KEY: &'static str = "own-refusals-fixture";
        const CONTENT_DEPENDENT: bool = true;
        const INTERACTIVE: bool = true;
        type Params = land_owners::LandOwnersParams;
        type Reply = crate::LandOwners;
        fn load(
            source: &Source,
            _params: &Self::Params,
            context: &LoadContext,
            _cancelled: &AtomicBool,
        ) -> std::io::Result<Self::Reply> {
            let catalogue = crate::catalogue::catalogue(source, context.content()?)?;
            catalogue.refuse("own reason", "OWN");
            Ok(crate::LandOwners {
                countries: Vec::new(),
                warnings: catalogue.refusal_lines(),
            })
        }
    }
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content tests");
    let root = std::env::temp_dir().join(format!("own-refusals-{}", uuid::Uuid::new_v4()));
    let game = root.join("game");
    dashboard_base::test_fixtures::snow_content::write(&game);
    let (source, context) = crate::test_fixtures::source_with_content(
        vec![crate::test_fixtures::world_frame(10, false)],
        &game,
    );
    // Another consumer's refusal, noted outside every reply.
    crate::catalogue::catalogue(&source, context.content().unwrap())
        .unwrap()
        .refuse("another consumer's reason", "OTHER");
    let reply = (register::<Refusing>().load)(
        &source,
        "lease",
        serde_json::json!({}),
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.contains("own reason"), "{reply}");
    assert!(!reply.contains("another consumer"), "{reply}");
    let _ = std::fs::remove_dir_all(&root);
}
