
use game_ui::assets::AssetResolver;
use game_ui::topbar::{flag_candidates_for_size, FlagSizeFolder};
use std::path::PathBuf;

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn resolve(resolver: &AssetResolver, tag: &str, ideology: &str, cosmetic: Option<&str>) -> String {
    for cand in flag_candidates_for_size(
        Some(tag),
        Some(ideology),
        cosmetic,
        None,
        FlagSizeFolder::Medium,
    ) {
        if resolver
            .resolve_optional_file(&cand)
            .expect("resolve")
            .is_some()
        {
            return cand;
        }
    }
    panic!("no flag resolved for {tag}/{ideology} cosmetic={cosmetic:?}");
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn flag_precedence_matches_shipped_inventory() {
    let root = game_root();
    let resolver = AssetResolver::new(vec![root]);

    assert_eq!(
        resolve(&resolver, "GER", "democratic", None),
        "gfx/flags/medium/GER_democratic.tga"
    );
    assert_eq!(
        resolve(&resolver, "SOV", "communism", None),
        "gfx/flags/medium/SOV_communism.tga"
    );
    assert_eq!(
        resolve(&resolver, "RAJ", "neutrality", None),
        "gfx/flags/medium/RAJ.tga"
    );
    assert_eq!(
        resolve(&resolver, "RAJ", "neutrality", Some("RAJ_UK")),
        "gfx/flags/medium/RAJ_UK.tga"
    );
}
