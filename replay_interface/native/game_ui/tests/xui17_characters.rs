use game_ui::assets::AssetResolver;
use game_ui::character_ui::{
    inspect_character_layouts, resolve_named_ace_portrait, CharacterPortraitCatalog,
    LeaderSkillCostCatalog, LeaderSkillKind,
};
use game_ui::gfx::SpriteRegistry;
use std::path::PathBuf;

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .or_else(|| std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    root.is_dir().then_some(root)
}

#[test]
fn installed_xui17_layouts_and_portrait_declarations_are_complete() {
    let Some(root) = game_root() else {
        eprintln!("SKIP XUI17 installed-file gate: the game root missing");
        return;
    };
    let resolver = AssetResolver::new(vec![root]);
    let gates = inspect_character_layouts(&resolver).expect("parse XUI17 GUI files");
    assert_eq!(gates.len(), 9);
    assert!(gates.iter().all(|gate| gate.complete()), "{gates:#?}");

    let portraits = CharacterPortraitCatalog::load(&resolver).expect("parse character portraits");
    assert!(
        portraits.token_count() > 100,
        "installed portrait catalog unexpectedly small: {}",
        portraits.token_count()
    );

    let costs = LeaderSkillCostCatalog::load(&resolver).expect("parse effective leader skills");
    assert_eq!(costs.cost(LeaderSkillKind::Army, 4), Some(1000));
    assert_eq!(costs.cost(LeaderSkillKind::Navy, 4), Some(800));

    let aces = resolver
        .resolve_optional_file("interface/aces.gfx")
        .expect("resolve aces.gfx")
        .expect("aces.gfx present");
    let mut sprites = SpriteRegistry::new();
    sprites
        .parse_file(&aces.path, &aces.rel_path)
        .expect("parse aces.gfx");
    let named = resolve_named_ace_portrait("FIN", "Ilmari", "Juutilainen", &sprites)
        .expect("documented FIN named ace portrait");
    assert_eq!(named.0, "GFX_FIN_ace_Ilmari_Juutilainen");
    assert!(named.1.ends_with("FIN_ilmari_juutilainen_ace.dds"));
}
