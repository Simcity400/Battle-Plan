use engine_sites::{Image, Located, OwnedImage};
use std::cell::Cell;

const BASE: usize = 0x1000_0000;
const PDATA: usize = 0x1000;

fn image_file() -> Vec<u8> {
    const PE: usize = 0x80;
    const OPTIONAL: usize = PE + 24;
    const SECTION: usize = OPTIONAL + 0xf0;
    const RAW: usize = 0x200;
    let mut file = vec![0; RAW + 12];
    file[..2].copy_from_slice(b"MZ");
    file[0x3c..0x40].copy_from_slice(&(PE as u32).to_le_bytes());
    file[PE..PE + 4].copy_from_slice(b"PE\0\0");
    file[PE + 6..PE + 8].copy_from_slice(&1_u16.to_le_bytes());
    file[PE + 20..PE + 22].copy_from_slice(&0xf0_u16.to_le_bytes());
    file[OPTIONAL..OPTIONAL + 2].copy_from_slice(&0x20b_u16.to_le_bytes());
    file[OPTIONAL + 24..OPTIONAL + 32].copy_from_slice(&(BASE as u64).to_le_bytes());
    file[OPTIONAL + 56..OPTIONAL + 60].copy_from_slice(&0x2000_u32.to_le_bytes());
    file[SECTION..SECTION + 6].copy_from_slice(b".pdata");
    file[SECTION + 8..SECTION + 12].copy_from_slice(&12_u32.to_le_bytes());
    file[SECTION + 12..SECTION + 16].copy_from_slice(&(PDATA as u32).to_le_bytes());
    file[SECTION + 16..SECTION + 20].copy_from_slice(&12_u32.to_le_bytes());
    file[SECTION + 20..SECTION + 24].copy_from_slice(&(RAW as u32).to_le_bytes());
    file
}

#[test]
fn an_unreadable_live_section_withholds_every_site_until_the_image_reads_again() {
    let file = image_file();
    let owned = OwnedImage::new(&file).unwrap();
    let on_disk = Located::locate(&owned.image(BASE), true);
    assert!(on_disk.layouts().is_ok());
    assert!(on_disk.summary().keys > 0);

    let refuse = Cell::new(false);
    let copy = |address: usize, out: &mut [u8]| {
        let offset = address - BASE;
        if offset == PDATA {
            if refuse.get() {
                return false;
            }
            out.copy_from_slice(&file[file.len() - 12..]);
        } else {
            out.copy_from_slice(&file[offset..offset + out.len()]);
        }
        true
    };
    let image = Image::live(BASE, &copy).unwrap();
    refuse.set(true);
    let partial = Located::locate(&image, true);
    assert_eq!(partial.summary().keys_verified, 0);
    assert_eq!(partial.withheld().count(), on_disk.summary().keys);
    assert!(partial.layouts().is_err());
    assert!(partial.withheld().all(|(_, reason)| reason
        .to_string()
        .contains("part of the running image could not be read")));

    refuse.set(false);
    let live = Located::locate(&image, true);
    assert!(live.layouts().is_ok());
    let reasons = |sites: &Located| {
        sites.withheld().map(|(site, reason)| (site, reason.to_string())).collect::<Vec<_>>()
    };
    assert_eq!(reasons(&live), reasons(&on_disk));
}
