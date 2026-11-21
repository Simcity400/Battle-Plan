use engine_sites::{Image, Located, OwnedImage, layout_checked};
use std::path::PathBuf;

#[test]
fn reference_build_verifies_every_key() {
    let dir = std::env::var_os("BATTLE_PLAN_GAME_DIR")
        .or_else(|| std::env::var_os("HOI4_GAME_DIR"))
        .expect("reference build tests require BATTLE_PLAN_GAME_DIR with the supported installation");
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

    let (mapped, writable) = mapped(&bytes);
    let hole = base + 0x1000;
    let refuse = std::cell::Cell::new(false);
    let copy = |address: usize, out: &mut [u8]| {
        let Some(at) = address.checked_sub(base) else { return false };
        let Some(end) = at.checked_add(out.len()) else { return false };
        let in_hole = address < hole + 0x1000 && address + out.len() > hole;
        let crosses_word = at / 8 != (end - 1) / 8;
        let mutable = writable.iter().any(|range| range.contains(&at)) && crosses_word;
        if (refuse.get() && in_hole) || mutable {
            return false;
        }
        let Some(source) = mapped.get(at..end) else { return false };
        out.copy_from_slice(source);
        true
    };
    let image = Image::live(base, &copy).unwrap();
    refuse.set(true);
    let partial = Located::locate(&image, true);
    assert_eq!(partial.summary().keys_verified, 0);
    assert_eq!(partial.withheld().count(), summary.keys);
    refuse.set(false);
    let live = Located::locate(&image, true);
    assert_eq!(live.moved().collect::<Vec<_>>(), checked.moved().collect::<Vec<_>>());
    assert_eq!(live.summary().keys_verified, summary.keys);

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

fn mapped(file: &[u8]) -> (Vec<u8>, Vec<std::ops::Range<usize>>) {
    let u16_at = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
    let pe = u32_at(0x3c);
    let headers = u32_at(pe + 24 + 60);
    let mut image = vec![0; u32_at(pe + 24 + 56)];
    image[..headers].copy_from_slice(&file[..headers]);
    let mut writable = Vec::new();
    let table = pe + 24 + u16_at(pe + 20);
    for index in 0..u16_at(pe + 6) {
        let row = table + 40 * index;
        let (virtual_size, rva, raw_size, raw) = (u32_at(row + 8), u32_at(row + 12), u32_at(row + 16), u32_at(row + 20));
        let len = raw_size.min(virtual_size);
        image[rva..rva + len].copy_from_slice(&file[raw..raw + len]);
        if u32_at(row + 36) & 0x8000_0000 != 0 {
            writable.push(rva..rva + virtual_size);
        }
    }
    (image, writable)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
