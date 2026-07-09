//! FILE-derived census gate for country-flag selection precedence.
//! Resolves flags against the SHIPPED `gfx/flags` inventory; skipped when the
//! the game install is absent.

use game_ui::assets::AssetResolver;
use game_ui::topbar::flag_candidates;
use std::path::PathBuf;

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    root.is_dir().then_some(root)
}

/// First existing candidate path for (tag, ideology, cosmetic).
fn resolve(resolver: &AssetResolver, tag: &str, ideology: &str, cosmetic: Option<&str>) -> String {
    for cand in flag_candidates(Some(tag), Some(ideology), cosmetic, None) {
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
fn flag_precedence_matches_shipped_inventory() {
    let Some(root) = game_root() else {
        eprintln!("SKIP flag_precedence_matches_shipped_inventory: the game root missing");
        return;
    };
    let resolver = AssetResolver::new(vec![root]);

    // GER has NO base GER.tga — the ideology variant must be selected.
    assert_eq!(
        resolve(&resolver, "GER", "democratic", None),
        "gfx/flags/GER_democratic.tga"
    );
    assert_eq!(
        resolve(&resolver, "SOV", "communism", None),
        "gfx/flags/SOV_communism.tga"
    );
    // RAJ neutrality with no ideology-specific flag falls back to the base tag.
    assert_eq!(
        resolve(&resolver, "RAJ", "neutrality", None),
        "gfx/flags/RAJ.tga"
    );
    // With the 1936 RAJ_UK cosmetic, the cosmetic base flag wins over the real
    // tag (RAJ_UK_neutrality is absent; RAJ.tga must NOT be selected).
    assert_eq!(
        resolve(&resolver, "RAJ", "neutrality", Some("RAJ_UK")),
        "gfx/flags/RAJ_UK.tga"
    );
}
