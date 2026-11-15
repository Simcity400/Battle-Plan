//! The Rust locator agrees with tools/sites.py on the executable in BATTLE_PLAN_GAME_DIR: on a
//! layout-checked build every key verifies with checked layouts, and by witnesses every key
//! but the vtables no constructor witnesses.
use engine_sites::{Located, OwnedImage, layout_checked};
use std::path::PathBuf;

#[test]
fn reference_build_verifies_every_key() {
    let Some(dir) = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")) else {
        eprintln!("BATTLE_PLAN_GAME_DIR unset; skipped");
        return;
    };
    let bytes = std::fs::read(PathBuf::from(dir).join("hoi4.exe")).unwrap();
    let sha: [u8; 32] = sha256(&bytes);
    if !layout_checked(&sha) {
        eprintln!("BATTLE_PLAN_GAME_DIR is not a layout-checked build; skipped");
        return;
    }
    let owned = OwnedImage::new(&bytes).unwrap();
    let base = 0x1_4000_0000;
    let checked = Located::locate(&owned.image(base), true);
    let withheld: Vec<_> = checked.withheld().map(|(_, w)| w.to_string()).collect();
    assert!(withheld.is_empty(), "{withheld:#?}");
    let summary = checked.summary();
    assert_eq!(summary.keys_verified, summary.keys);

    let witnessed = Located::locate(&owned.image(base), false);
    let withheld: Vec<_> = witnessed.withheld().map(|(_, w)| w.to_string()).collect();
    let other: Vec<_> = withheld
        .iter()
        .filter(|w| !w.contains("no constructor witnesses its class layout"))
        .collect();
    assert!(other.is_empty(), "{other:#?}");
    let summary = witnessed.summary();
    assert_eq!(summary.keys_verified + withheld.len(), summary.keys);
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
