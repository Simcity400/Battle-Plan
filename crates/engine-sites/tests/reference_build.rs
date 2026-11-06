//! The Rust locator agrees with tools/sites.py on the executable in BATTLE_PLAN_GAME_DIR: on the
//! reference build every key verifies with checked layouts and by witnesses alike.
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
        eprintln!("BATTLE_PLAN_GAME_DIR is not the reference build; skipped");
        return;
    }
    let owned = OwnedImage::new(&bytes).unwrap();
    let base = 0x1_4000_0000;
    for checked in [true, false] {
        let located = Located::locate(&owned.image(base), checked);
        let withheld: Vec<_> = located.withheld().map(|(_, w)| w.to_string()).collect();
        assert!(withheld.is_empty(), "{withheld:#?}");
        let summary = located.summary();
        assert_eq!(summary.keys_verified, summary.keys);
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
