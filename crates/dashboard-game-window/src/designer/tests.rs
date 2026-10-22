//! The designer drawn from synthetic recorded facts over the installed content and over
//! synthetic mod layers.

use std::collections::BTreeMap;

use installed_content_core::ResolvedLayers;

use super::{slots::Cell, *};
use crate::{
    products::game_window::SlotState as Slot,
    tests::{folder, installed, tables, tga},
};

/// A filled cell holding `definition`, or a cell in `state` holding nothing.
fn drawn(slot: [i32; 2], state: SlotState, definition: Option<&str>) -> DrawnCell {
    DrawnCell {
        cell: Cell {
            slot,
            state,
            definition: definition.map(|_| (1, false)),
        },
        definition: definition.map(|key| Ok(key.to_owned())),
        hover: definition.map(|key| Hover {
            title: key.to_owned(),
            details: Vec::new(),
        }),
    }
}

/// An ordinary template: a column of three `regiment`s with its addable and locked rows, one
/// `support` company with its addable row, and the next empty regiment column locked.
fn designer(regiment: &str, support: &str) -> Designer {
    Designer {
        hq: false,
        name: Ok("Synthetic Division".to_owned()),
        grids: Ok(vec![
            DrawnGrid {
                kind: GridKind::Regiments,
                cells: vec![
                    drawn([0, 0], SlotState::Filled, Some(regiment)),
                    drawn([0, 1], SlotState::Filled, Some(regiment)),
                    drawn([0, 2], SlotState::Filled, Some(regiment)),
                    drawn([0, 3], SlotState::Addable, None),
                    drawn([0, 4], SlotState::Locked, None),
                    drawn([1, 0], SlotState::Addable, None),
                    drawn([2, 0], SlotState::Blocked, None),
                ],
                refused: vec!["the country's brigade height unlock is not recorded".into()],
            },
            DrawnGrid {
                kind: GridKind::Support,
                cells: vec![
                    drawn([0, 0], SlotState::Filled, Some(support)),
                    drawn([0, 1], SlotState::Addable, None),
                ],
                refused: Vec::new(),
            },
            DrawnGrid {
                kind: GridKind::RegimentalSupport,
                cells: Vec::new(),
                refused: Vec::new(),
            },
        ]),
        symbol: Ok(Symbol {
            candidates: vec![(unit_glyph(regiment), glyph_frame(false))],
        }),
        priority: Ok((1, 3)),
        regiments: true,
        paradrop: Ok(false),
        equipment_button: Ok(EQUIPMENT_BUTTON),
        experience: Ok("Army experience: 12".to_owned()),
        stats: vec![
            rows::Row {
                grid: rows::Grid::Base,
                label: Ok("Speed".to_owned()),
                value: Err("not proven".to_owned()),
            },
            rows::Row {
                grid: rows::Grid::Combat,
                label: Ok("Soft attack".to_owned()),
                value: Ok("12.0".to_owned()),
            },
        ],
        summary: (
            [
                Ok("80%".to_owned()),
                Err("not proven".to_owned()),
                Err("not proven".to_owned()),
                Err("not proven".to_owned()),
            ],
            Ok("18".to_owned()),
        ),
        basis: vec!["The country's unit bonuses as read on 1944.2.11".to_owned()],
        read_hour: None,
    }
}

fn draw_on(layers: &ResolvedLayers, designer: &Designer) -> Bound {
    draw_with_session(layers, designer).0
}

/// [`draw_on`], with the session it drew from.
fn draw_with_session(
    layers: &ResolvedLayers,
    designer: &Designer,
) -> (
    Bound,
    std::sync::Arc<std::sync::Mutex<crate::session::Session>>,
) {
    let localisation = installed_content_localisation::localisation::read(layers, "english").unwrap();
    let sessions = std::sync::Mutex::new(crate::session::Sessions::default());
    let session = crate::session::session(
        &sessions,
        "installed",
        layers,
        "english",
        installed_content_image::native::Decode::default(),
        [1920, 1080],
        crate::session::Opener::Request,
        &|| true,
    )
    .unwrap()
    .unwrap();
    let bound = super::draw(
        &mut session.lock().unwrap(),
        layers,
        &localisation,
        designer,
        false,
        &BTreeMap::new(),
    )
    .unwrap();
    (bound, session)
}

/// The sizes of the pages the drawn designer samples.
fn pages(
    drawn: &(
        Bound,
        std::sync::Arc<std::sync::Mutex<crate::session::Session>>,
    ),
) -> Vec<[u32; 2]> {
    crate::tests::drawn_pages(drawn.0.scene.as_ref().unwrap(), &drawn.1.lock().unwrap())
}

fn slot<'a>(bound: &'a Bound, name: &str) -> &'a WindowSlot {
    bound
        .slots
        .iter()
        .find(|slot| slot.name == name)
        .unwrap_or_else(|| panic!("no {name} slot in {:?}", bound.slots))
}

/// The installed designer lays out its composition cells in their grids, draws the editor's
/// buttons as the game draws them with no pending edit, explains the filled cells and the
/// buttons on hover and names what it does not bind.
#[test]
fn the_installed_designer_draws_its_composition() {
    tables();
    let layers = installed(&[]);
    let bound = draw_on(&layers, &designer("infantry", "engineer"));
    let scene = bound.scene.as_ref().unwrap();
    assert!(!scene.parts.is_empty());
    let technique = |control: &str| -> Vec<Option<String>> {
        scene
            .parts
            .iter()
            .filter(|part| part.path.ends_with(control))
            .map(|part| {
                part.shading
                    .compiled
                    .as_ref()
                    .map(|effect| effect.technique.clone())
            })
            .collect()
    };
    // Reset and save draw disabled; duplicate, whose state is not recorded, is hidden; the
    // equipment button acts with regiments, so it draws Up.
    for control in [".reset_button", ".save_button"] {
        let drawn = technique(control);
        assert!(!drawn.is_empty(), "{control}");
        assert!(
            drawn
                .iter()
                .all(|technique| technique.as_deref() == Some("Disable")),
            "{control}: {drawn:?}"
        );
    }
    assert!(technique(".duplicate_button").is_empty());
    assert!(
        technique(".division_equipment_btn")
            .iter()
            .all(|technique| technique.as_deref() != Some("Disable"))
    );
    let read_only: Vec<_> = scene
        .hovers
        .iter()
        .filter(|hover| hover.title == "Read-only")
        .collect();
    assert_eq!(read_only.len(), 2, "{:?}", scene.hovers);
    // Four filled cells and both stat rows, each dated by its inputs' read.
    let scene_hovers: Vec<_> = scene
        .hovers
        .iter()
        .filter(|hover| hover.title != "Read-only")
        .collect();
    assert_eq!(scene_hovers.len(), 6, "{:?}", scene.hovers);
    assert!(scene.hovers.iter().any(|hover| hover.title == "Speed"
        && hover.details
            == [
                "Unavailable: not proven",
                "The country's unit bonuses as read on 1944.2.11"
            ]));
    assert_eq!(slot(&bound, "stat rows").state, Slot::Refused);
    assert_eq!(
        slot(&bound, "summary stats and combat width").state,
        Slot::Refused
    );
    assert_eq!(
        slot(&bound, "unit glyphs").state,
        Slot::Bound,
        "{:?}",
        bound.slots
    );
    assert_eq!(slot(&bound, "composition").state, Slot::Refused);
    assert_eq!(
        slot(&bound, "controls").state,
        Slot::Bound,
        "{:?}",
        bound.slots
    );
    assert_eq!(bound.title.as_deref(), Some("Synthetic Division"));
    // Cells keep their slots: the filled regiment column's hovers stack down one column.
    let mut regiments: Vec<_> = scene
        .hovers
        .iter()
        .filter(|hover| hover.title == "infantry")
        .collect();
    regiments.sort_by(|a, b| a.rect[1].total_cmp(&b.rect[1]));
    assert_eq!(regiments.len(), 3);
    assert!(
        regiments
            .windows(2)
            .all(|pair| pair[0].rect[0] == pair[1].rect[0] && pair[0].rect[1] < pair[1].rect[1])
    );
    // A definition without a declared glyph is named, never drawn as another unit.
    let bound = draw_on(&layers, &designer("synthetic_missing_unit", "engineer"));
    let glyphs = slot(&bound, "unit glyphs");
    assert_eq!(glyphs.state, Slot::Refused);
    assert!(
        glyphs
            .detail
            .as_deref()
            .unwrap()
            .contains("GFX_unit_synthetic_missing_unit_icon_medium is not declared")
    );
}

/// A mod's own sub-unit draws its own glyph from the mod's texture, and a mod that replaces
/// the designer's file draws its own designer; one that renames a grid the binding needs is
/// refused that control by name while the rest still draws.
#[test]
fn mods_supply_units_and_replace_the_designer() {
    tables();
    let unit = folder(
        "designer_unit",
        &[
            (
                "interface/synthetic_units.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_unit_synthetic_ranger_icon_medium" texturefile = "gfx/interface/synthetic_ranger.tga" noOfFrames = 2 } }"#.to_vec(),
            ),
            ("gfx/interface/synthetic_ranger.tga", tga(78, 26)),
        ],
    );
    let layers = installed(&[&unit]);
    let drawn = draw_with_session(&layers, &designer("synthetic_ranger", "engineer"));
    assert_eq!(
        slot(&drawn.0, "unit glyphs").state,
        Slot::Bound,
        "{:?}",
        drawn.0.slots
    );
    assert!(pages(&drawn).contains(&[78, 26]), "{:?}", pages(&drawn));
    let replaced = |name: &str, grid: &str| {
        folder(
            name,
            &[(
                "interface/divisiondesignerview.gui",
                format!(
                    r#"guiTypes = {{
                containerWindowType = {{ name = "countrydivisiondesignerview" size = {{ width = 640 height = 480 }}
                    containerWindowType = {{ name = "subunits" size = {{ width = 600 height = 400 }}
                        gridboxtype = {{ name = "{grid}" position = {{ x = 10 y = 10 }} size = {{ width = 400 height = 300 }} slotsize = {{ width = 80 height = 40 }} format = "UPPER_LEFT" }}
                        gridboxtype = {{ name = "support_grid" position = {{ x = 450 y = 10 }} size = {{ width = 80 height = 300 }} slotsize = {{ width = 80 height = 30 }} format = "UPPER_LEFT" }}
                        gridboxtype = {{ name = "regiment_support_grid" position = {{ x = 10 y = 320 }} size = {{ width = 400 height = 40 }} slotsize = {{ width = 80 height = 30 }} format = "UPPER_LEFT" }} }}
                    containerWindowType = {{ name = "non_hq_view" size = {{ width = 600 height = 60 }}
                        containerWindowType = {{ name = "divisions" size = {{ width = 300 height = 30 }}
                            instantTextboxType = {{ name = "name" font = "hoi_16mbs" maxWidth = 280 maxHeight = 20 text = "placeholder" }} }} }}
                    containerWindowType = {{ name = "hq_view" size = {{ width = 600 height = 60 }} }} }}
                containerWindowType = {{ name = "designer_division_regiment_slot_entry" size = {{ width = 80 height = 40 }}
                    iconType = {{ name = "Background" spriteType = "GFX_div_unit_item_bg_wide" }}
                    iconType = {{ name = "unit_icon" spriteType = "GFX_locked_icon" }}
                    iconType = {{ name = "invisible_icon" spriteType = "GFX_locked_icon" }}
                    instantTextboxType = {{ name = "name" font = "hoi_16mbs" text = "placeholder" }} }}
                containerWindowType = {{ name = "designer_division_support_slot_entry" size = {{ width = 80 height = 30 }}
                    iconType = {{ name = "Background" spriteType = "GFX_div_subunit_item_bg" }}
                    iconType = {{ name = "unit_icon" spriteType = "GFX_locked_icon" }}
                    iconType = {{ name = "invisible_icon" spriteType = "GFX_locked_icon" }}
                    instantTextboxType = {{ name = "name" font = "hoi_16mbs" text = "placeholder" }} }} }}"#
                )
                .into_bytes(),
            )],
        )
    };
    let own = replaced("designer_own", "regiments_grid");
    let layers = installed(&[&own]);
    let bound = draw_on(&layers, &designer("infantry", "engineer"));
    let scene = bound.scene.as_ref().unwrap();
    assert_eq!(scene.size, [640, 480]);
    assert_eq!(scene.hovers.len(), 4);
    // The replaced file has no header controls beyond the name: each is refused by path.
    let controls = slot(&bound, "controls");
    assert_eq!(controls.state, Slot::Refused);
    assert!(
        controls
            .detail
            .as_deref()
            .unwrap()
            .contains("div_templ_symbol_button")
    );
    let renamed = replaced("designer_renamed", "battalion_grid");
    let layers = installed(&[&renamed]);
    let bound = draw_on(&layers, &designer("infantry", "engineer"));
    // The regiments are not drawn; the support company still is.
    assert_eq!(bound.scene.as_ref().unwrap().hovers.len(), 1);
    assert!(
        slot(&bound, "controls")
            .detail
            .as_deref()
            .unwrap()
            .contains("has no subunits/regiments_grid control")
    );
    let refused = super::check(&installed_content_core::gui::native::read_layers(&layers).unwrap());
    assert!(
        refused
            .iter()
            .any(|why| why.contains("subunits/regiments_grid")),
        "{refused:?}"
    );
}

/// `tga`'s pixels in a one-level, 32-bit uncompressed DDS.
fn dds(tga: &[u8]) -> Vec<u8> {
    let width = u32::from(u16::from_le_bytes([tga[12], tga[13]]));
    let height = u32::from(u16::from_le_bytes([tga[14], tga[15]]));
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124_u32),
        (8, 0x100F),
        (12, height),
        (16, width),
        (20, width * 4),
        (28, 1),
        (76, 32),
        (80, 0x41),
        (88, 32),
        (92, 0xFF_0000),
        (96, 0xFF00),
        (100, 0xFF),
        (104, 0xFF00_0000),
        (108, 0x1000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&tga[18..]);
    bytes
}

/// A bitmap font file whose one page holds every printable ASCII glyph.
fn fnt() -> Vec<u8> {
    let mut text = String::from(
        "info face=\"synthetic\" size=16\ncommon lineHeight=16 base=13 scaleW=37 scaleH=41 pages=1\n\
         page id=0 file=\"synthetic.dds\"\nchars count=95\n",
    );
    for id in 32..127 {
        text.push_str(&format!(
            "char id={id} x=0 y=0 width=4 height=6 xoffset=0 yoffset=0 xadvance=5 page=0 chnl=15\n"
        ));
    }
    text.into_bytes()
}

/// A mod that replaces a designer sprite's texture, or the font its stat rows use, draws the
/// designer in its own style: the drawing samples the mod's texture and the mod's glyph page,
/// never the installation's.
#[test]
fn a_mods_sprites_and_fonts_restyle_the_designer() {
    tables();
    let textures = pages;
    let installed_only = draw_with_session(&installed(&[]), &designer("infantry", "engineer"));
    let restyle = folder(
        "designer_restyle",
        &[
            // The regiment cells' background, at the path the installed sprite names.
            ("gfx/interface/div_unit_item_bg_1.dds", dds(&tga(98, 34))),
            // The stat rows' font: its glyphs and its page.
            ("gfx/fonts/hoi4_typewriter16_inverted.fnt", fnt()),
            (
                "gfx/fonts/hoi4_typewriter16_inverted.dds",
                dds(&tga(37, 41)),
            ),
        ],
    );
    let restyled = draw_with_session(&installed(&[&restyle]), &designer("infantry", "engineer"));
    for size in [[98, 34], [37, 41]] {
        assert!(
            !textures(&installed_only).contains(&size),
            "{:?}",
            textures(&installed_only)
        );
        assert!(
            textures(&restyled).contains(&size),
            "{size:?} not drawn: {:?}",
            textures(&restyled)
        );
    }
}

/// The same restyle selected as a visual mod over the installed game, not recorded with it, styles
/// the designer too: the session reads the stack's presentation.
#[test]
fn a_selected_visual_mod_restyles_the_designer() {
    use installed_content_core::content::visual;
    tables();
    let restyle = folder(
        "designer_visual_restyle",
        &[
            ("gfx/interface/div_unit_item_bg_1.dds", dds(&tga(98, 34))),
            ("gfx/fonts/hoi4_typewriter16_inverted.fnt", fnt()),
            (
                "gfx/fonts/hoi4_typewriter16_inverted.dds",
                dds(&tga(37, 41)),
            ),
            // The window's own text, replaced.
            (
                "localisation/replace/designer_visual_l_english.yml",
                "\u{feff}l_english:\n DESIGNER_ARMY_EXPERIENCE:0 \"Styled $VALUE$\"\n"
                    .as_bytes()
                    .to_vec(),
            ),
        ],
    );
    let user = folder(
        "designer_visual_user",
        &[(
            "mod/restyle.mod",
            format!(
                "name=\"Restyle\"\npath=\"{}\"\n",
                restyle.display().to_string().replace('\\', "/")
            )
            .into_bytes(),
        )],
    );
    let mut layers = installed(&[]);
    let options = installed_content_core::ResolveOptions {
        game_dir: user.clone(),
        user_dir: user.clone(),
        cache_dir: user.join("cache"),
    };
    let choice = visual::Choice {
        id: "mod/restyle.mod".into(),
        name: "Restyle".into(),
    };
    let plan = visual::apply(&mut layers, &options, &[choice], &[], &Default::default()).unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let textures = pages(&draw_with_session(
        &layers,
        &designer("infantry", "engineer"),
    ));
    for size in [[98, 34], [37, 41]] {
        assert!(textures.contains(&size), "{size:?} not drawn: {textures:?}");
    }
    // The text the designer formats is the selected mod's; the recorded text stays recorded.
    let catalogue = dashboard_text::catalogue::Catalogue::of_layers(layers, "english").unwrap();
    let shown = catalogue.presentation_localisation().unwrap();
    let styled = super::experience_text(&shown, 100_000_000).unwrap();
    assert!(styled.starts_with("Styled "), "{styled}");
    let recorded = super::experience_text(&catalogue.localisation, 100_000_000).unwrap();
    assert!(!recorded.starts_with("Styled "), "{recorded}");
}

#[test]
fn header_frames_and_sprites_follow_the_template() {
    assert_eq!(priority_frames(0, 3), [4, 2, 3]);
    assert_eq!(priority_frames(1, 3), [1, 5, 3]);
    assert_eq!(priority_frames(2, 4), [1, 2, 7, 4]);
    let mut settings = TemplateSettings {
        allow_new_equipment: 1,
        allow_foreign_equipment: 1,
        forbidden_equipment: Some(Vec::new()),
        ..TemplateSettings::default()
    };
    assert_eq!(equipment_button(&settings, &[]), Ok(EQUIPMENT_BUTTON));
    assert_eq!(
        equipment_button(&settings, &[9]),
        Ok(EQUIPMENT_BUTTON_RESTRICTED)
    );
    settings.allow_foreign_equipment = 0;
    assert_eq!(
        equipment_button(&settings, &[]),
        Ok(EQUIPMENT_BUTTON_RESTRICTED)
    );
    settings.forbidden_equipment = None;
    assert!(equipment_button(&settings, &[]).is_err());
    assert_eq!(experience_points(1_234_567), 12);
    assert_eq!(experience_points(-150_000), -1);
}

/// The installed designer paints as the game draws it, every part drawn.
#[test]
#[ignore = "requires DXC and a GPU; reads the installation"]
fn the_installed_designer_paints() {
    tables();
    let (bound, session) = draw_with_session(&installed(&[]), &designer("infantry", "engineer"));
    let mut session = session.lock().unwrap();
    let session = &mut *session;
    let scene = bound
        .scene
        .unwrap()
        .paint(crate::compose::Canvas {
            sheets: &session.gui,
            pages: &session.images,
            painter: &mut session.painter,
        })
        .unwrap();
    assert!(scene.image.starts_with("data:image/png;base64,"));
    assert!(
        !scene.notes.iter().any(|note| note.contains("incompletely")),
        "{:?}",
        scene.notes
    );
}
