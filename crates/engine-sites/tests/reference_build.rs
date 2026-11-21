use engine_sites::{Image, Located, OwnedImage, layout_checked};
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

    let mapped = mapped(&bytes);
    let copy = |address: usize, out: &mut [u8]| {
        let Some(at) = address.checked_sub(base) else { return false };
        let Some(source) = at.checked_add(out.len()).and_then(|end| mapped.get(at..end)) else { return false };
        out.copy_from_slice(source);
        true
    };
    let live = Located::locate(&Image::live(base, &copy).unwrap(), true);
    assert_eq!(live.moved().collect::<Vec<_>>(), checked.moved().collect::<Vec<_>>());
    assert_eq!(live.summary().keys_verified, summary.keys);

    let hole = base + 0x1000;
    let holed = |address: usize, out: &mut [u8]| {
        address.checked_add(out.len()).is_some_and(|end| end <= hole || address >= hole + 0x1000) && copy(address, out)
    };
    let partial = Located::locate(&Image::live(base, &holed).unwrap(), true);
    assert_eq!(partial.summary().keys_verified, 0);
    assert_eq!(partial.withheld().count(), summary.keys);

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

fn mapped(file: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
    let pe = u32_at(0x3c);
    let headers = u32_at(pe + 24 + 60);
    let mut image = vec![0; u32_at(pe + 24 + 56)];
    image[..headers].copy_from_slice(&file[..headers]);
    let table = pe + 24 + u16_at(pe + 20);
    for index in 0..u16_at(pe + 6) {
        let row = table + 40 * index;
        let (virtual_size, rva, raw_size, raw) = (u32_at(row + 8), u32_at(row + 12), u32_at(row + 16), u32_at(row + 20));
        let len = raw_size.min(virtual_size);
        image[rva..rva + len].copy_from_slice(&file[raw..raw + len]);
    }
    image
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
