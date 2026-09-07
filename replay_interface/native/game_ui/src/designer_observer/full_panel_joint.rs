use super::tests::{admitted_production_renderer, production_fixture, TestRoot};
use crate::production_renderer::ProductionUiRenderer;
use std::fs;

pub(super) const REGIONAL_RGBA: [u8; 4] = [30, 80, 220, 255];
pub(super) const DEFAULT_RGBA: [u8; 4] = [20, 180, 70, 255];
pub(super) const FLAG_RGBA: [u8; 4] = [220, 180, 20, 255];

pub(super) fn authored_fixture() -> (TestRoot, ProductionUiRenderer) {
    let (root, _) = production_fixture();
    for directory in ["common/country_tags", "map", "history/states",
        "gfx/interface/equipmentdesigner/graphic_db"] {
        fs::create_dir_all(root.0.join(directory)).unwrap();
    }
    for (path, text) in [
        ("common/units/equipment/designer.txt",
            "equipments={ ship_hull={ is_archetype=yes type=screen_ship module_slots={ main={} } } }"),
        ("interface/designer_window.gui",
            r#"guiTypes={ containerWindowType={ name="equipment_designer_ship_hull" size={ width=1 height=1 } } }"#),
        ("common/country_tags/tags.txt", r#"GER="countries/Germany.txt""#),
        ("map/default.map", r#"continent="continent.txt" definitions="definition.csv" provinces="provinces.bmp""#),
        ("map/continent.txt", "continents={ europe asia africa }"),
        ("history/states/template.txt", "state={id=17 provinces={1}}"),
        ("history/states/slot.txt", "state={id=1 provinces={2}}"),
        ("history/states/runtime.txt", "state={id=73 provinces={3}}"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;land;false;plains;2\n3;3;0;0;land;false;plains;3\n"),
        ("gfx/interface/equipmentdesigner/graphic_db/joint.txt",
            "europe={ ship_hull={ pool={ icons={ gfx/interface/joint_regional.tga } weight=2 } } } default={ ship_hull={ pool={ icons={ gfx/interface/joint_default.tga } weight=1 } } }"),
    ] {
        fs::write(root.0.join(path), text).unwrap();
    }
    let mut bitmap = vec![0u8; 66];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[10..14].copy_from_slice(&54u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&4i32.to_le_bytes());
    bitmap[22..26].copy_from_slice(&1i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24u16.to_le_bytes());
    for (pixel, red) in bitmap[54..].chunks_exact_mut(3).zip([1, 2, 3, 1]) {
        pixel.copy_from_slice(&[0, 0, red]);
    }
    fs::write(root.0.join("map/provinces.bmp"), bitmap).unwrap();
    let template = fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
    assert_eq!(&template[..18], &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 0x20]);
    assert_eq!(template.len(), 26);
    for (path, [red, green, blue, alpha]) in [
        ("gfx/interface/joint_regional.tga", REGIONAL_RGBA),
        ("gfx/interface/joint_default.tga", DEFAULT_RGBA),
        ("gfx/flags/small/GER_neutrality.tga", FLAG_RGBA),
    ] {
        let mut texture = template.clone();
        for pixel in texture[18..].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[blue, green, red, alpha]);
        }
        fs::write(root.0.join(path), texture).unwrap();
    }
    let font_path = root.0.join("gfx/fonts/xui13_test.fnt");
    let mut font = fs::read_to_string(&font_path).unwrap();
    for character in "Vessel".chars().collect::<std::collections::BTreeSet<_>>() {
        let prefix = format!("char id={} ", u32::from(character));
        if !font.lines().any(|line| line.starts_with(&prefix)) {
            font.push_str(&format!("{prefix}x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n"));
        }
    }
    fs::write(font_path, font).unwrap();
    let renderer = admitted_production_renderer(&root.0, None);
    (root, renderer)
}
