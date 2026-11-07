//! Synthetic windows through the native layout, the painting and the policy rows.

use std::collections::BTreeMap;

use installed_content_core::gui::native::read;
use viewer_foundation::map::{
    native_gui::{PartKind, Resources},
    test_support::gui::{Fixture, fixture_with},
};

use crate::{
    compose::{self, BoundList, Hover, Laid, Row},
    mio,
};

pub(crate) fn tables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
}

/// `root` composed with `rows` row windows, each hovered, in a list `list` of `row`.
fn composed(
    fixture: &Fixture,
    root: &str,
    list: Option<(&str, &str, usize)>,
    scroll: &compose::Scroll,
) -> Result<Laid, Vec<String>> {
    let resources: &dyn Resources = fixture;
    let template = resources.gui_type(root.as_bytes()).unwrap();
    let instance = compose::construct(template, [800, 600], resources).unwrap();
    let lists = match list {
        Some((path, row, count)) => {
            let row = resources.gui_type(row.as_bytes()).unwrap();
            vec![BoundList {
                list: template.path_named(path).unwrap(),
                template: row,
                rows: (0..count)
                    .map(|index| Row {
                        instance: compose::construct(row, [800, 600], resources).unwrap(),
                        hover: Some(Hover {
                            title: format!("row {index}"),
                            details: Vec::new(),
                        }),
                    })
                    .collect(),
            }]
        }
        None => Vec::new(),
    };
    compose::compose(
        &compose::Window {
            name: root,
            root: template,
            instance: &instance,
            lists: &lists,
            grids: &[],
            hovers: &[],
        },
        scroll,
        resources,
    )
}

/// Every part the native layout gives the window is handed to the paint, in draw order, at the
/// place the layout gives it: a sprite at its frame, a text with its glyphs.
#[test]
fn every_laid_out_part_is_painted_in_draw_order() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = { containerWindowType = {
            name = "w" size = { width = 100 height = 50 } clipping = yes
            iconType = { name = "i" spriteType = "GFX_four" frame = 3 position = { x = 5 y = 6 } }
            instantTextBoxType = { name = "t" font = "Main_14" text = "ab" position = { x = 1 y = 2 }
                maxWidth = 60 maxHeight = 20 }
        } }"#,
    ));
    let laid = composed(&fixture, "w", None, &BTreeMap::new()).unwrap();
    assert_eq!(laid.size, [100, 50]);
    let paths: Vec<_> = laid.parts.iter().map(|part| part.path.as_str()).collect();
    assert_eq!(paths, ["w.i", "w.t"]);
    let PartKind::Sprite {
        geometry, offsets, ..
    } = &laid.parts[0].kind
    else {
        panic!("{:?}", laid.parts[0].kind)
    };
    assert_eq!(geometry.corners()[0], [5.0, 6.0]);
    assert_eq!(offsets[0], 0.5);
    assert!(matches!(&laid.parts[1].kind, PartKind::Text { glyphs, .. } if glyphs.len() == 2));
    assert!(laid.notes.is_empty(), "{:?}", laid.notes);
}

/// A window whose container `c` holds grid `g` of 20-pixel rows, laid out in `format` with the
/// grid at `grid_y`, composed with five rows in its first column at `offset` of `c`.
fn scrolled_grid(format: &str, grid_y: i32, offset: Option<i32>) -> (Laid, String) {
    let fixture = fixture_with(read(
        format!(
            r#"guiTypes = {{
            extendedScrollbarType = {{ name = "sb" size = {{ width = 10 height = 50 }}
                background = {{ name = "bg" spriteType = "GFX_even" }}
                track = {{ name = "track" spriteType = "GFX_bar3x17" }}
                slider = {{ name = "slider" spriteType = "GFX_bar3x17" }}
                decreaseButton = {{ name = "dec" spriteType = "GFX_four" }}
                increaseButton = {{ name = "inc" spriteType = "GFX_four" position = {{ x = -10 y = -10 }} }} }}
            containerWindowType = {{ name = "w" size = {{ width = 200 height = 200 }}
                containerWindowType = {{ name = "c" size = {{ width = 100 height = 60 }}
                    verticalScrollbar = "sb" clipping = yes
                    background = {{ name = "back" spriteType = "GFX_even" }}
                    gridboxtype = {{ name = "g" position = {{ x = 0 y = {grid_y} }}
                        size = {{ width = 100 height = 40 }}
                        slotsize = {{ width = 100 height = 20 }} format = "{format}" }} }} }}
            containerWindowType = {{ name = "item" size = {{ width = 100 height = 20 }}
                iconType = {{ name = "i" spriteType = "GFX_four" }} }}
        }}"#
        )
        .as_bytes(),
    ));
    let resources: &dyn Resources = &fixture;
    let template = resources.gui_type(b"w").unwrap();
    let item = resources.gui_type(b"item").unwrap();
    let instance = compose::construct(template, [800, 600], resources).unwrap();
    let container = template.path_named("c").unwrap().path.clone();
    let grids = [compose::BoundGrid {
        grid: template.path_named("c/g").unwrap(),
        template: item,
        items: (0..5)
            .map(|row| {
                (
                    Row {
                        instance: compose::construct(item, [800, 600], resources).unwrap(),
                        hover: Some(Hover {
                            title: format!("item {row}"),
                            details: Vec::new(),
                        }),
                    },
                    [0, row],
                )
            })
            .collect(),
    }];
    let scroll: compose::Scroll = offset
        .map(|offset| ((container.clone(), false), offset))
        .into_iter()
        .collect();
    let laid = compose::compose(
        &compose::Window {
            name: "w",
            root: template,
            instance: &instance,
            lists: &[],
            grids: &grids,
            hovers: &[],
        },
        &scroll,
        resources,
    )
    .unwrap();
    (laid, container)
}

fn tops(laid: &Laid) -> Vec<f32> {
    laid.hovers.iter().map(|hover| hover.rect[1]).collect()
}

/// A grid whose items overflow its scrolling container makes the container scroll as the game
/// does: the scene offers it as a scrolling area over its range, its offset moves the items and
/// their hovers, clamped to that range, and each item's hover names the area, so focusing it
/// scrolls that area.
#[test]
fn a_grid_overflowing_its_container_scrolls_it() {
    tables();
    let (rest, container) = scrolled_grid("UPPER_LEFT", 0, None);
    // Five 20-pixel items in a 60-pixel container: it scrolls over 40 pixels.
    let area = rest
        .lists
        .iter()
        .find(|list| list.path == container)
        .unwrap();
    assert_eq!((area.offset, area.max, area.horizontal), (0, 40, false));
    assert!(
        rest.hovers
            .iter()
            .all(|hover| hover.list.as_deref() == Some(container.as_str()))
    );
    let (scrolled, _) = scrolled_grid("UPPER_LEFT", 0, Some(30));
    assert_eq!(scrolled.lists[0].offset, 30);
    for (rest, scrolled) in tops(&rest).iter().zip(tops(&scrolled)) {
        assert_eq!(rest - scrolled, 30.0);
    }
    // Past its range it stops at the far end.
    assert_eq!(
        scrolled_grid("UPPER_LEFT", 0, Some(99)).0.lists[0].offset,
        40
    );
    // Each item's hover is clipped to the container.
    assert!(scrolled.hovers.iter().all(|hover| hover.clip.is_some()));
}

/// A grid that overflows its container upward gives a range below zero: the scene's offsets run
/// from that range's near end, so the content at rest is the far end, and the near end shows
/// the rows above.
#[test]
fn a_grid_overflowing_upward_scrolls_below_zero() {
    tables();
    let (rest, container) = scrolled_grid("LOWER_LEFT", 20, None);
    let area = rest
        .lists
        .iter()
        .find(|list| list.path == container)
        .unwrap();
    assert!(area.max > 0, "{area:?}");
    assert_eq!(area.offset, area.max, "{area:?}");
    let (near, _) = scrolled_grid("LOWER_LEFT", 20, Some(0));
    assert_eq!(near.lists[0].offset, 0);
    for (rest, near) in tops(&rest).iter().zip(tops(&near)) {
        assert_eq!(near - rest, area.max as f32);
    }
}

/// A hovered root control explains itself over its own region; others explain nothing.
#[test]
fn root_controls_are_explained_by_path() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = { containerWindowType = {
            name = "w" size = { width = 100 height = 50 }
            iconType = { name = "i" spriteType = "GFX_four" position = { x = 5 y = 6 } }
            iconType = { name = "j" spriteType = "GFX_four" position = { x = 40 y = 6 } }
        } }"#,
    ));
    let resources: &dyn Resources = &fixture;
    let template = resources.gui_type(b"w").unwrap();
    let instance = compose::construct(template, [800, 600], resources).unwrap();
    let hovered = template.path_named("j").unwrap().path.clone();
    let laid = compose::compose(
        &compose::Window {
            name: "w",
            root: template,
            instance: &instance,
            lists: &[],
            grids: &[],
            hovers: &[(
                hovered.as_str(),
                Hover {
                    title: "j".into(),
                    details: Vec::new(),
                },
            )],
        },
        &BTreeMap::new(),
        resources,
    )
    .unwrap();
    assert_eq!(laid.parts.len(), 2);
    assert_eq!(laid.hovers.len(), 1);
    assert_eq!(laid.hovers[0].rect[0], 40.0);
}

/// A smooth list box places its rows at the scroll offset, clamped to the rows' run beyond the
/// box, and each row's hover moves with it.
#[test]
fn list_rows_scroll_within_their_run() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = {
            containerWindowType = { name = "w" size = { width = 200 height = 100 }
                smoothListBoxType = { name = "l" position = { x = 10 y = 10 } size = { x = 100 y = 50 } }
            }
            containerWindowType = { name = "row" size = { width = 100 height = 20 }
                iconType = { name = "i" spriteType = "GFX_even" }
            }
        }"#,
    ));
    let at = |offset: i32| {
        let scroll = BTreeMap::from([(("w.l".to_owned(), false), offset)]);
        composed(&fixture, "w", Some(("l", "row", 4)), &scroll).unwrap()
    };
    let top = at(0);
    assert_eq!(top.lists.len(), 1);
    assert_eq!((top.lists[0].offset, top.lists[0].max), (0, 30));
    assert_eq!(top.hovers.len(), 4);
    assert_eq!(top.hovers[1].title, "row 1");
    let scrolled = at(500);
    assert_eq!(scrolled.lists[0].offset, 30);
    assert_eq!(
        scrolled.hovers[0].rect[1],
        top.hovers[0].rect[1] - 30.0,
        "{:?}",
        scrolled.hovers
    );
    // Every row's parts are clipped to the list.
    let clipped = scrolled
        .parts
        .iter()
        .filter(|part| part.clip.is_some())
        .count();
    assert_eq!(clipped, 4);
}

/// A standard list box scrolls by whole rows: the scene scrolls it over the pixels down to the
/// last row its bar reaches, and an offset shows the rows from the one it reaches, where the
/// first rows were drawn.
#[test]
fn standard_list_rows_scroll_by_whole_rows() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = {
            scrollbarType = { name = "lsb" size = { x = 10 y = 60 } minValue = 0 maxValue = 1
                slider = "s" track = "t" leftButton = "d" rightButton = "u"
                buttonType = { name = "u" spriteType = "GFX_four" }
                buttonType = { name = "d" spriteType = "GFX_four" }
                guiButtonType = { name = "t" spriteType = "GFX_bar3x17" }
                guiButtonType = { name = "s" spriteType = "GFX_bar3x17" } }
            containerWindowType = { name = "w" size = { width = 200 height = 100 }
                listBoxType = { name = "l" position = { x = 10 y = 10 } size = { x = 100 y = 50 }
                    scrollbarType = "lsb" }
            }
            containerWindowType = { name = "row" size = { width = 100 height = 20 }
                iconType = { name = "i" spriteType = "GFX_even" }
            }
        }"#,
    ));
    let at = |offset: i32| {
        let scroll = BTreeMap::from([(("w.l".to_owned(), false), offset)]);
        composed(&fixture, "w", Some(("l", "row", 5)), &scroll).unwrap()
    };
    let titles = |laid: &Laid| -> Vec<String> {
        laid.hovers.iter().map(|hover| hover.title.clone()).collect()
    };
    // Five 20-pixel rows of which two fit in 50: the bar reaches the fourth, 60 pixels down.
    let top = at(0);
    assert_eq!(top.lists.len(), 1);
    let list = &top.lists[0];
    assert_eq!((list.offset, list.max, list.horizontal), (0, 60, false));
    assert_eq!(titles(&top), ["row 0", "row 1"]);
    let scrolled = at(45);
    assert_eq!(scrolled.lists[0].offset, 45);
    assert_eq!(titles(&scrolled), ["row 2", "row 3"]);
    assert_eq!(scrolled.hovers[0].rect, top.hovers[0].rect);
    let end = at(500);
    assert_eq!(end.lists[0].offset, 60);
    assert_eq!(titles(&end), ["row 3", "row 4"]);
}

/// A missing root is refused by the window's name, not drawn from another template.
#[test]
fn a_window_whose_root_draws_nothing_is_refused() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = { containerWindowType = { name = "w" size = { width = 10 height = 10 }
            iconType = { name = "i" spriteType = "GFX_undefined_here" } } }"#,
    ));
    // An unknown sprite draws the fallback; the window itself still lays out.
    assert!(composed(&fixture, "w", None, &BTreeMap::new()).is_ok());
}

/// Rows are the organisation's allowed policies in their recorded order, joined to the
/// country's instances; a token the roster lacks has no row and is named.
#[test]
fn policy_rows_join_the_allowed_tokens() {
    use replay_data::rows::{IndustrialOrganisationDetailsRow, IndustrialOrganisationPolicy};
    let policy = |token: u32, cost: i32| IndustrialOrganisationPolicy {
        token,
        cost,
        days: 30,
        country: 7,
        name: format!("Policy {token}").into_bytes(),
        cached_score_raw: 0,
    };
    let mut details = IndustrialOrganisationDetailsRow {
        allowed_policies: vec![3, 2, 1],
        ..Default::default()
    };
    details.state.policy = 1;
    let roster = [policy(1, 50), policy(3, 75)];
    let spell = |token: u32| Ok(format!("token_{token}").into_bytes());
    let tag = |country: u32| {
        if country == 7 {
            Ok("SYN".to_owned())
        } else {
            Err(format!("country {country} has no tag"))
        }
    };
    let (rows, unlisted) = mio::rows(&details, &roster, spell, tag).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.token.clone()).collect::<Vec<_>>(),
        [b"token_3".to_vec(), b"token_1".to_vec()]
    );
    assert_eq!(rows[0].cost, 75);
    // Each row keeps the tag of the country its instance belongs to.
    assert_eq!(rows[0].tag.as_deref(), Ok("SYN"));
    assert!(!rows[0].selected && rows[1].selected);
    assert_eq!(unlisted, [b"token_2".to_vec()]);
    // The constructor's sentinel selects nothing, even if a policy had that token.
    details.state.policy = 0x4c17;
    details.allowed_policies = vec![0x4c17];
    let (rows, _) = mio::rows(&details, &[policy(0x4c17, 1)], spell, tag).unwrap();
    assert!(!rows[0].selected);
    let failed = mio::rows(&details, &roster, |_| Err("unspelled".to_owned()), tag);
    assert_eq!(failed.unwrap_err(), "unspelled");
}

/// A policy whose visibility does not hold has no row and one whose visibility is unknown is
/// withheld by name; availability is kept per row, unknown with its reason; each row keeps its
/// definition's authored icon, empty when it names none.
#[test]
fn policy_definitions_decide_what_a_row_can_show() {
    tables();
    use installed_content_core::{
        industrial_organisations::policies::Catalog, key_names::KeyNames, parser::Item,
    };
    let mut policies = Catalog::default();
    policies
        .read_file(
            b"shown = { icon = GFX_shown available = { always = no } }
              hidden = { icon = GFX_hidden visible = { always = no } }
              undecided = { icon = GFX_undecided visible = { has_war = yes } }
              unknown = { icon = GFX_unknown available = { has_war = yes } }
              bare = { }",
            &KeyNames::default(),
        )
        .unwrap();
    // A stand-in for the trigger interpreter: `always` decides, anything else is unknown.
    let gate = |items: &[Item]| {
        let item = &items[0];
        if item.key.as_deref() == Some(b"always".as_slice()) {
            Ok(item.value.scalar() == Some(b"yes".as_slice()))
        } else {
            Err("not recorded".to_owned())
        }
    };
    let row = |token: &str| mio::PolicyRow {
        token: token.as_bytes().to_vec(),
        name: format!("{token} name").into_bytes(),
        cost: 1,
        days: 2,
        selected: token == "bare",
        tag: Ok("SYN".into()),
    };
    let rows = [
        "shown",
        "hidden",
        "undecided",
        "unknown",
        "bare",
        "undefined",
    ]
    .map(row);
    let stored = |bytes: &[u8]| {
        if bytes.starts_with(b"bare") {
            Err("stored in another language".to_owned())
        } else {
            Ok(String::from_utf8_lossy(bytes).into_owned())
        }
    };
    let (shown, withheld) = mio::shown(rows.to_vec(), Ok(&policies), gate, stored);
    assert_eq!(
        withheld
            .iter()
            .map(|(token, _)| token.as_str())
            .collect::<Vec<_>>(),
        ["undecided", "undefined"]
    );
    assert_eq!(withheld[0].1, "not recorded");
    assert!(withheld[1].1.contains("is not defined"), "{withheld:?}");
    assert_eq!(
        shown
            .iter()
            .map(|row| row.policy.token.as_slice())
            .collect::<Vec<_>>(),
        [b"shown".as_slice(), b"unknown", b"bare"]
    );
    assert_eq!(shown[0].available, Ok(false));
    assert_eq!(shown[1].available, Err("not recorded".to_owned()));
    assert_eq!(shown[2].available, Ok(true));
    assert_eq!(shown[0].icon, b"GFX_shown");
    assert!(shown[2].icon.is_empty());
    let hover = mio::hover(&shown[2]);
    // A name the viewer may not show leaves the row and its other facts; the key titles it.
    assert!(shown[2].name.is_err());
    assert_eq!(hover.title, "bare");
    assert!(hover.details.iter().any(|line| line == "Key: bare"));
    assert!(hover.details.iter().any(|line| line.contains("Attached")));
    assert!(
        mio::hover(&shown[0])
            .details
            .iter()
            .any(|line| line.starts_with("Not available"))
    );
    assert!(
        mio::hover(&shown[1])
            .details
            .iter()
            .any(|line| line == "Availability unknown: not recorded")
    );
}

/// A folder of synthetic files for a content layer.
pub(crate) fn folder(name: &str, files: &[(&str, Vec<u8>)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("hoi4_game_window_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    root
}

/// A 32-bit uncompressed Targa of `width` by `height` opaque white texels.
pub(crate) fn tga(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = 32;
    bytes[17] = 8;
    bytes.resize(18 + usize::from(width) * usize::from(height) * 4, 0xff);
    bytes
}

/// The installation's layers, with `overlay` folders mounted over it.
pub(crate) fn installed(overlay: &[&std::path::Path]) -> installed_content_core::ResolvedLayers {
    let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let mut folders = vec![(game.as_path(), &[][..])];
    folders.extend(overlay.iter().map(|path| (*path, &[][..])));
    installed_content_core::ResolvedLayers::from_folders(&folders).unwrap()
}

/// The smallest stack a session opens over, in its own `name`d folder: just the map size.
fn bare(name: &str) -> installed_content_core::ResolvedLayers {
    let mut bmp = vec![0_u8; 66];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&66_u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&4_i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1_i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    let root = folder(
        name,
        &[
            (
                "map/default.map",
                br#"definitions = "definition.csv" provinces = "provinces.bmp" terrain = "terrain.bmp" rivers = "rivers.bmp" heightmap = "heightmap.bmp" tree_definition = "trees.bmp""#
                    .to_vec(),
            ),
            ("map/provinces.bmp", bmp),
        ],
    );
    installed_content_core::ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap()
}

/// `count` synthetic rows, each named, with `icon` (or none of its own), the first attached and
/// the second unavailable.
fn synthetic_rows(count: usize, icon: Option<&[u8]>) -> Vec<mio::Shown> {
    (0..count)
        .map(|index| mio::Shown {
            policy: mio::PolicyRow {
                token: format!("synthetic_policy_{index}").into_bytes(),
                name: format!("Synthetic policy {index}").into_bytes(),
                cost: 10,
                days: 30,
                selected: index == 0,
                tag: Ok("SYN".into()),
            },
            name: Ok(format!("Synthetic policy {index}")),
            icon: icon.map(<[u8]>::to_vec).unwrap_or_default(),
            available: Ok(index != 1),
        })
        .collect()
}

/// An organisation whose own conditions hold and whose cooldown is over, its quotes unscaled
/// (a base cost of 10 quotes 10) and its owner's political power unknown, so an unattached
/// available row no other part blocks is undecided.
const OPEN: mio::Gate = mio::Gate {
    organisation: Ok(true),
    cooldown_days: 0,
    cost: Ok(dashboard_base::products::industrial_organisations::PolicyCost {
        local: 0,
        country: 0,
    }),
    political_power_raw: Err(String::new()),
};

type Drawn = (
    crate::products::game_window::Bound,
    std::sync::Arc<std::sync::Mutex<crate::session::Session>>,
);

fn drawn(
    layers: &installed_content_core::ResolvedLayers,
    rows: &[mio::Shown],
    scroll: &compose::Scroll,
) -> Drawn {
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
    let bound = mio::draw(
        &mut session.lock().unwrap(),
        layers,
        &localisation,
        rows,
        &[],
        &OPEN,
        scroll,
    )
    .unwrap();
    (bound, session)
}

/// The sizes of the texture pages `laid`'s parts sample, from `session`'s sheets and pages.
pub(crate) fn drawn_pages(laid: &Laid, session: &crate::session::Session) -> Vec<[u32; 2]> {
    use viewer_foundation::map::native_gui::{InlineSource, Sheets};
    let sheet = |sprite: &[u8]| session.gui.sheet(sprite).map(|sheet| sheet.texture);
    let mut ids = Vec::new();
    for part in &laid.parts {
        match &part.kind {
            PartKind::Sprite { sprite, .. } | PartKind::Tiles { sprite, .. } => {
                ids.extend(sheet(sprite))
            }
            PartKind::Cell { texture, .. } => ids.push(*texture),
            PartKind::Inline { source, .. } => match source {
                InlineSource::Sprite(sprite) => ids.extend(sheet(sprite)),
                InlineSource::Texture(texture) => ids.push(*texture),
            },
            PartKind::Text { glyphs, .. } => ids.extend(glyphs.iter().map(|glyph| glyph.texture)),
            _ => {}
        }
    }
    ids.iter()
        .filter_map(|id| session.images.get(*id))
        .map(|page| [page.width, page.height])
        .collect()
}

/// Whether `laid` draws from a texture page of `size`.
fn loaded(
    laid: &Laid,
    session: &std::sync::Mutex<crate::session::Session>,
    size: [u32; 2],
) -> bool {
    drawn_pages(laid, &session.lock().unwrap()).contains(&size)
}

/// The technique a part draws with.
fn technique(part: &viewer_foundation::map::native_gui::Part) -> Option<&str> {
    part.shading
        .compiled
        .as_ref()
        .map(|effect| effect.technique.as_str())
}

/// The installed policy window lays out through the native layout with its content's own
/// sprites, fonts and list, and its rows scroll. The attached policy, removable, draws its
/// background in frame 2 and its icon in Up and hides its cost; an unattached policy that is not
/// available draws both in their Disable technique; a row whose gate is not decided (an
/// unattached policy whose owner's political power is unknown) draws neither, counted. Every
/// unattached row shows its positive quote through the content's cost text. A policy with no
/// icon of its own takes the game's lookup, ending at the unknown idea's sprite.
#[test]
fn the_installed_policy_window_draws_and_scrolls() {
    tables();
    let layers = installed(&[]);
    let rows = synthetic_rows(12, None);
    let (bound, _) = drawn(&layers, &rows, &BTreeMap::new());
    let laid = bound.scene.unwrap();
    assert!(
        laid.parts
            .iter()
            .any(|part| matches!(part.kind, PartKind::Text { .. }))
    );
    assert!(!laid.hovers.is_empty() && laid.hovers.len() <= rows.len());
    assert_eq!(laid.lists.len(), 1);
    let list = &laid.lists[0];
    assert!(list.max > 0, "{list:?}");
    let drawn_of = |control: &str| -> Vec<_> {
        laid.parts
            .iter()
            .filter(|part| part.path.ends_with(control))
            .collect()
    };
    // The attached and the blocked rows draw their background and icon: Up, then Disable.
    for control in [".background_button", ".icon"] {
        let parts = drawn_of(control);
        let techniques: Vec<_> = parts.iter().map(|part| technique(part)).collect();
        assert_eq!(techniques, [Some("Up"), Some("Disable")], "{control}");
    }
    assert!(drawn_of(".icon").iter().all(|icon| matches!(&icon.kind,
        PartKind::Sprite { sprite, .. } if sprite == b"GFX_idea_unknown")));
    // The attached row's background takes its second frame; the blocked row's, the first.
    let frames: Vec<_> = drawn_of(".background_button")
        .iter()
        .map(|part| match &part.kind {
            PartKind::Sprite { offsets, .. } => offsets[0] > 0.0 || offsets[1] > 0.0,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(frames, [true, false]);
    assert!(drawn_of(".ai_value").is_empty());
    // Unattached rows show their quote through the content's cost text; the attached row
    // alone shows none.
    assert!(!drawn_of(".attach_cost").is_empty());
    let (attached, _) = drawn(&layers, &rows[..1], &BTreeMap::new());
    assert!(
        !attached
            .scene
            .unwrap()
            .parts
            .iter()
            .any(|part| part.path.ends_with(".attach_cost"))
    );
    let slot = |name: &str| {
        bound
            .slots
            .iter()
            .find(|slot| slot.name == name)
            .unwrap()
            .state
    };
    use crate::products::game_window::SlotState;
    assert_eq!(slot("row icon"), SlotState::Bound);
    assert_eq!(slot("row attach cost"), SlotState::Bound);
    assert_eq!(slot("row state"), SlotState::Refused);
    // At the end of its run the first rows have left the list.
    let path = list.path.clone();
    let (bound, _) = drawn(&layers, &rows, &BTreeMap::from([((path, false), i32::MAX)]));
    let scrolled = bound.scene.unwrap();
    assert_eq!(scrolled.lists[0].offset, scrolled.lists[0].max);
    assert!(scrolled.hovers[0].rect[1] < laid.hovers[0].rect[1]);
}

/// The installed window paints as the game draws it: an image of its size, every part drawn.
#[test]
#[ignore = "requires DXC and a GPU; reads the installation"]
fn the_installed_policy_window_paints() {
    tables();
    let layers = installed(&[]);
    let (bound, session) = drawn(&layers, &synthetic_rows(4, None), &BTreeMap::new());
    let mut session = session.lock().unwrap();
    let session = &mut *session;
    let scene = bound
        .scene
        .unwrap()
        .paint(compose::Canvas {
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

/// A mod layer's own sprite draws as a policy icon from the mod's texture, and a mod that
/// renames a control the binding needs refuses the window by that name.
#[test]
fn mod_layers_supply_icons_and_replace_templates() {
    tables();
    let icons = folder(
        "icons",
        &[
            (
                "interface/synthetic_policy_icons.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_synthetic_policy_icon" texturefile = "gfx/interface/synthetic_policy_icon.tga" } }"#.to_vec(),
            ),
            ("gfx/interface/synthetic_policy_icon.tga", tga(36, 20)),
        ],
    );
    let layers = installed(&[&icons]);
    // The second row is blocked, so its icon draws.
    let (bound, session) = drawn(
        &layers,
        &synthetic_rows(2, Some(b"GFX_synthetic_policy_icon")),
        &BTreeMap::new(),
    );
    let laid = bound.scene.unwrap();
    assert!(loaded(&laid, &session, [36, 20]), "the mod's icon texture");
    // A mod that replaces the file declaring the window draws its own window.
    let replaced = |name: &str, control: &str| {
        folder(
            name,
            &[(
                "interface/military_industrial_organization/industrial_org_policy.gui",
                format!(
                    r#"guiTypes = {{
                containerWindowType = {{ name = "industrial_organisation_policy_window" size = {{ width = 300 height = 200 }}
                    containerWindowType = {{ name = "policy_list_window" size = {{ width = 300 height = 200 }}
                        smoothListBoxType = {{ name = "policy_list" size = {{ x = 280 y = 180 }} }} }} }}
                containerWindowType = {{ name = "policy_item" size = {{ width = 280 height = 40 }}
                    buttonType = {{ name = "background_button" spriteType = "GFX_mio_entry_bg" }}
                    iconType = {{ name = "icon" }} instantTextboxType = {{ name = "{control}" }}
                    instantTextboxType = {{ name = "attach_cost" }}
                    instantTextboxType = {{ name = "ai_value" }} }} }}"#
                )
                .into_bytes(),
            )],
        )
    };
    let own = replaced("own", "name");
    let layers = installed(&[&own]);
    let (bound, _) = drawn(&layers, &synthetic_rows(2, None), &BTreeMap::new());
    assert_eq!(bound.scene.unwrap().size, [300, 200]);
    // One that renames a control the binding needs refuses the window by that name.
    let renamed = replaced("renamed", "renamed_name");
    let layers = installed(&[&renamed]);
    let (bound, _) = drawn(&layers, &synthetic_rows(2, None), &BTreeMap::new());
    let refused = bound.scene.unwrap_err();
    assert!(
        refused
            .iter()
            .any(|why| why.contains("has no name control")),
        "{refused:?}"
    );
}

/// A visual mod mounted as the stack's presentation styles the window (its own template), and
/// a sprite whose texture it breaks is the recording's own, named on the window that drew it and
/// on every later window of the session.
#[test]
fn visual_mods_style_windows_and_what_they_break_is_drawn_from_the_recording_by_name() {
    use installed_content_core::content::{observe_overlay_refusals, visual};
    tables();
    let icon = "gfx/interface/synthetic_policy_icon.tga";
    let recorded = folder(
        "recorded_icons",
        &[
            (
                "interface/synthetic_policy_icons.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_synthetic_policy_icon" texturefile = "gfx/interface/synthetic_policy_icon.tga" } }"#.to_vec(),
            ),
            (icon, tga(36, 20)),
        ],
    );
    let art = folder(
        "visual_art",
        &[
            (icon, b"not a texture".to_vec()),
            (
                "interface/military_industrial_organization/industrial_org_policy.gui",
                br#"guiTypes = {
                containerWindowType = { name = "industrial_organisation_policy_window" size = { width = 300 height = 200 }
                    containerWindowType = { name = "policy_list_window" size = { width = 300 height = 200 }
                        smoothListBoxType = { name = "policy_list" size = { x = 280 y = 180 } } } }
                containerWindowType = { name = "policy_item" size = { width = 280 height = 40 }
                    buttonType = { name = "background_button" spriteType = "GFX_mio_entry_bg" }
                    iconType = { name = "icon" } instantTextboxType = { name = "name" }
                    instantTextboxType = { name = "attach_cost" }
                    instantTextboxType = { name = "ai_value" } } }"#
                    .to_vec(),
            ),
        ],
    );
    let user = folder(
        "visual_user",
        &[(
            "mod/art.mod",
            format!(
                "name=\"Art\"\npath=\"{}\"\n",
                art.display().to_string().replace('\\', "/")
            )
            .into_bytes(),
        )],
    );
    let mut layers = installed(&[&recorded]);
    let options = installed_content_core::ResolveOptions {
        game_dir: user.clone(),
        user_dir: user.clone(),
        cache_dir: user.join("cache"),
    };
    let choice = visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    };
    let plan = visual::apply(&mut layers, &options, &[choice], &[], &Default::default()).unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let rows = synthetic_rows(2, Some(b"GFX_synthetic_policy_icon"));
    let ((bound, session), fallbacks) =
        observe_overlay_refusals(|| drawn(&layers, &rows, &BTreeMap::new()));
    let laid = bound.scene.unwrap();
    assert_eq!(laid.size, [300, 200], "the visual mod's own window");
    assert!(loaded(&laid, &session, [36, 20]), "the recording's icon");
    assert!(
        fallbacks
            .iter()
            // The sprite is the recording's own declaration, drawn from the recording.
            .any(|refusal| refusal.starts_with("sprite GFX_synthetic_policy_icon (")
                && refusal.contains(&format!("texture {icon} ("))),
        "{fallbacks:?}"
    );
    let localisation = installed_content_localisation::localisation::read(&layers, "english").unwrap();
    let (_, again) = observe_overlay_refusals(|| {
        mio::draw(
            &mut session.lock().unwrap(),
            &layers,
            &localisation,
            &rows,
            &[],
            &OPEN,
            &BTreeMap::new(),
        )
        .unwrap()
    });
    assert_eq!(
        again, fallbacks,
        "a later window of the session names it again"
    );
}

/// A session over the visual mods' templates whose recording's own interface sprites cannot be
/// read (a recorded file listed when the mods were mounted, a folder when read) says so, by
/// name, on every window it draws, rather than quietly retrying each file by its path.
#[test]
fn a_recorded_registry_that_cannot_be_read_is_named_on_every_window() {
    use installed_content_core::content::visual;
    tables();
    let declared = br#"spriteTypes = { spriteType = { name = "GFX_synthetic_policy_icon" texturefile = "gfx/interface/synthetic_policy_icon.tga" } }"#;
    let recorded = folder(
        "unreadable_recorded_icons",
        &[
            ("interface/synthetic_policy_icons.gfx", declared.to_vec()),
            ("gfx/interface/synthetic_policy_icon.tga", tga(36, 20)),
        ],
    );
    // The mod's file of that path replaces the recording's in what its windows show.
    let art = folder(
        "unreadable_visual_art",
        &[("interface/synthetic_policy_icons.gfx", declared.to_vec())],
    );
    let user = folder(
        "unreadable_visual_user",
        &[(
            "mod/art.mod",
            format!(
                "name=\"Art\"\npath=\"{}\"\n",
                art.display().to_string().replace('\\', "/")
            )
            .into_bytes(),
        )],
    );
    let mut layers = installed(&[&recorded]);
    let options = installed_content_core::ResolveOptions {
        game_dir: user.clone(),
        user_dir: user.clone(),
        cache_dir: user.join("cache"),
    };
    let choice = visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    };
    visual::apply(&mut layers, &options, &[choice], &[], &Default::default()).unwrap();
    let file = recorded.join("interface/synthetic_policy_icons.gfx");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(&file).unwrap();
    let rows = synthetic_rows(1, Some(b"GFX_synthetic_policy_icon"));
    let (_, session) = drawn(&layers, &rows, &BTreeMap::new());
    let notes = session.lock().unwrap().notes.clone();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("the recording's own interface sprites and fonts cannot be read"),
        "{notes:?}"
    );
}

/// A request that passes its check and pauses before publishing never retires a newer
/// request's session: the check and the publication are one step under the sessions' lock, so
/// the newer request, admitted while the first is paused, publishes after it and its session
/// survives. A cancelled request, or one whose presentation the source has left, never becomes
/// current.
#[test]
fn a_paused_request_never_retires_a_newer_requests_session() {
    use crate::session::{Opener, Sessions, current, is_current, session};
    use std::sync::{
        Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    tables();
    let layers = bare("bare_paused");
    let open = |sessions: &Mutex<Sessions>, stack: &str, still: &dyn Fn() -> bool| {
        session(
            sessions,
            stack,
            &layers,
            "english",
            installed_content_image::native::Decode::default(),
            [1920, 1080],
            Opener::Request,
            still,
        )
        .unwrap()
        .unwrap()
    };
    let sessions = Mutex::new(Sessions::default());
    // A's presentation is the source's while A checks it; the selection then moves to B.
    let shows_a = AtomicBool::new(true);
    let (passed, resume) = (Barrier::new(2), Barrier::new(2));
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            open(&sessions, "a", &|| {
                let still = shows_a.load(Ordering::SeqCst);
                // A has passed its check: it pauses here, before publishing.
                passed.wait();
                resume.wait();
                still
            })
        });
        passed.wait();
        // While A is paused, the selection moves to B and B's request is admitted.
        shows_a.store(false, Ordering::SeqCst);
        let b = scope.spawn(|| open(&sessions, "b", &|| !shows_a.load(Ordering::SeqCst)));
        // B cannot publish while A holds the step it paused in.
        assert!(sessions.try_lock().is_err());
        resume.wait();
        let a = a.join().unwrap().lock().unwrap().id.clone();
        let b = b.join().unwrap().lock().unwrap().id.clone();
        (a, b)
    });
    assert!(is_current(&sessions, "b") && !is_current(&sessions, "a"));
    assert!(current(&sessions, &b).is_some(), "B's session survives");
    assert!(
        current(&sessions, &a).is_none(),
        "A's was retired by the newer B"
    );
    // A request cancelled, or admitted under a presentation the source has left, answers from a
    // session of its own and never becomes current.
    let cancelled = open(&sessions, "a", &|| false);
    let cancelled = cancelled.lock().unwrap().id.clone();
    assert!(current(&sessions, &cancelled).is_none());
    assert!(is_current(&sessions, "b") && current(&sessions, &b).is_some());
}

/// A request admitted under folders the source's latest work is no longer bound to is obsolete
/// though no visual mod is selected and its presentation is unchanged: paused after its check,
/// it cannot retire the session of the request admitted under the new folders, and admitted
/// again after them it answers privately.
#[test]
fn a_request_of_replaced_folders_never_retires_the_new_folders_session() {
    use crate::{
        products::game_window::still_current,
        session::{Opener, Sessions, current, is_current, session},
    };
    use dashboard_base::{
        catalogue::binding,
        products::ContentOptions,
        test_fixtures::{CAMPAIGN, records, source as frame},
    };
    use std::sync::{Barrier, Mutex};
    tables();
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "folders".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(
            replay::campaign::Replay::from_records(
                CAMPAIGN.into(),
                &[records(1, vec![frame(1, 100, Some(1))])],
            )
            .unwrap(),
        ),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let folders = |user: &str| ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: "game".into(),
            user_dir: user.into(),
            cache_dir: "cache".into(),
        },
        allow_unverified: false,
        presentation: Default::default(),
    };
    let old = folders("old user");
    let stack = |options: &ContentOptions| format!("{:?}", binding(options));
    let layers = bare("bare_replaced_folders");
    let open = |sessions: &Mutex<Sessions>, options: &ContentOptions, still: &dyn Fn() -> bool| {
        session(
            sessions,
            &stack(options),
            &layers,
            "english",
            installed_content_image::native::Decode::default(),
            [1920, 1080],
            Opener::Request,
            still,
        )
        .unwrap()
        .unwrap()
    };
    let sessions = Mutex::new(Sessions::default());
    source.bind(
        Some((&old.options, old.allow_unverified)),
        Default::default(),
    );
    let old = source.admit(dashboard_base::source::Work::Ahead).unwrap();
    let (passed, resume) = (Barrier::new(2), Barrier::new(2));
    let new = std::sync::OnceLock::new();
    let newer = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            open(&sessions, &old, &|| {
                let still = still_current(&source, &old, None);
                passed.wait();
                resume.wait();
                still
            })
        });
        passed.wait();
        // While A is paused, the folders change and B is admitted under them.
        source.bind(
            Some((&folders("new user").options, false)),
            Default::default(),
        );
        let admitted =
            new.get_or_init(|| source.admit(dashboard_base::source::Work::Ahead).unwrap());
        let b = scope.spawn(|| {
            open(&sessions, admitted, &|| {
                still_current(&source, admitted, None)
            })
        });
        resume.wait();
        a.join().unwrap();
        b.join().unwrap().lock().unwrap().id.clone()
    });
    let new = new.get().unwrap();
    assert!(is_current(&sessions, &stack(new)));
    assert!(current(&sessions, &newer).is_some(), "B's session survives");
    // A request of the old folders admitted before the change, uncancelled, answers privately.
    assert!(!still_current(&source, &old, None));
    let late = open(&sessions, &old, &|| still_current(&source, &old, None));
    let late = late.lock().unwrap().id.clone();
    assert!(current(&sessions, &late).is_none());
    assert!(is_current(&sessions, &stack(new)) && current(&sessions, &newer).is_some());
    // A cancelled request of the current folders never becomes current either.
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    assert!(!still_current(&source, new, Some(&cancelled)));
}

/// The monitors are read per-monitor DPI aware on any thread (a test thread is not), and the
/// thread keeps its own awareness afterwards.
#[cfg(windows)]
#[test]
fn monitors_are_read_dpi_aware_on_any_thread() {
    use windows_sys::Win32::UI::HiDpi::{
        GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
    };
    // SAFETY: both calls take and return plain awareness values.
    let awareness =
        || unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    let before = awareness();
    if let Err(why) = crate::session::monitors() {
        assert!(!why.to_string().contains("DPI-aware"), "{why}");
    }
    assert_eq!(awareness(), before);
}

/// A session holds its stack, and the stack does not hold the session: once its owner drops it,
/// the session (its templates, textures and encoded pages) is freed while the stack lives on.
/// Opening a session of another stack drops the earlier stack's.
#[test]
fn sessions_are_freed_with_their_owner_not_kept_by_their_stack() {
    tables();
    let layers = bare("bare_freed_with_owner");
    let open = |sessions: &std::sync::Mutex<crate::session::Sessions>, stack: &str| {
        crate::session::session(
            sessions,
            stack,
            &layers,
            "english",
            installed_content_image::native::Decode::default(),
            [1920, 1080],
            crate::session::Opener::Request,
            &|| true,
        )
        .unwrap()
        .unwrap()
    };
    let sessions = std::sync::Mutex::new(crate::session::Sessions::default());
    let first = std::sync::Arc::downgrade(&open(&sessions, "first"));
    assert!(first.upgrade().is_some(), "kept by its owner");
    let id = first.upgrade().unwrap().lock().unwrap().id.clone();
    assert!(crate::session::current(&sessions, &id).is_some());
    let second = std::sync::Arc::downgrade(&open(&sessions, "second"));
    assert!(
        first.upgrade().is_none(),
        "another stack's session drops the first"
    );
    assert!(crate::session::current(&sessions, &id).is_none());
    drop(sessions);
    assert!(
        second.upgrade().is_none(),
        "freed with its owner while the stack lives"
    );
    drop(layers);
}

/// A read-ahead of the content a source opened with, still between its stages when a window's
/// request opens the session of newer content, neither starts another stage nor publishes its
/// session over the request's: the request's session stays, and its artwork stays reachable.
#[test]
fn a_read_ahead_of_replaced_content_never_replaces_a_request_s_session() {
    use crate::session::{Opener, Sessions, current, is_current, session};
    use std::sync::{Arc, Barrier, Mutex};
    tables();
    let layers = Arc::new(bare("bare_read_ahead"));
    let open_while =
        |sessions: &Mutex<Sessions>, stack: &str, opener: Opener, current: &dyn Fn() -> bool| {
            session(
                sessions,
                stack,
                &layers,
                "english",
                installed_content_image::native::Decode::default(),
                [1920, 1080],
                opener,
                current,
            )
            .unwrap()
        };
    let open = |sessions: &Mutex<Sessions>, stack: &str, opener: Opener| {
        open_while(sessions, stack, opener, &|| true)
    };
    // The publication itself is guarded: once a request made `new` current, the read-ahead of
    // `old` gets nothing and leaves the request's session in place.
    let sessions = Mutex::new(Sessions::default());
    let requested = open(&sessions, "new", Opener::Request).unwrap();
    let id = requested.lock().unwrap().id.clone();
    assert!(open(&sessions, "old", Opener::ReadAhead).is_none());
    assert!(current(&sessions, &id).is_some());
    assert!(!is_current(&sessions, "old") && is_current(&sessions, "new"));
    // A request admitted under visual mods the source has since replaced answers from a session
    // of its own, which neither retires the current one nor becomes current. Whether it still
    // holds what the source shows is decided under the sessions' lock, so no request can pass
    // the check, pause, and then publish over a newer request's session.
    let late = open_while(&sessions, "old", Opener::Request, &|| {
        assert!(
            sessions.try_lock().is_err(),
            "decided under the sessions' lock"
        );
        false
    })
    .expect("a late request still answers");
    let late = late.lock().unwrap().id.clone();
    assert!(
        current(&sessions, &late).is_none(),
        "nothing kept for the late request"
    );
    assert!(current(&sessions, &id).is_some());
    assert!(!is_current(&sessions, "old") && is_current(&sessions, "new"));
    // And the stages: the read-ahead of `old` is inside its first stage when the request
    // opens `new`; its next stage, which would open `old`'s session, never starts.
    let sessions = Arc::new(Mutex::new(Sessions::default()));
    let (entered, release) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
    let published = Arc::new(Mutex::new(None));
    let ahead = {
        let (sessions, entered, release, published, layers) = (
            Arc::clone(&sessions),
            Arc::clone(&entered),
            Arc::clone(&release),
            Arc::clone(&published),
            Arc::clone(&layers),
        );
        std::thread::spawn(move || {
            let first = |_: &Mutex<Sessions>| {
                entered.wait();
                release.wait();
                true
            };
            let second = |sessions: &Mutex<Sessions>| {
                let opened = session(
                    sessions,
                    "old",
                    &layers,
                    "english",
                    installed_content_image::native::Decode::default(),
                    [1920, 1080],
                    Opener::ReadAhead,
                    &|| true,
                )
                .unwrap();
                *published.lock().unwrap() = Some(opened.is_some());
                true
            };
            #[allow(clippy::type_complexity)]
            let steps: [&dyn Fn(&Mutex<Sessions>) -> bool; 2] = [&first, &second];
            crate::products::game_window::stages(&steps, || {
                Some(Arc::clone(&sessions)).filter(|sessions| is_current(sessions, "old"))
            });
        })
    };
    entered.wait();
    let requested = open(&sessions, "new", Opener::Request).unwrap();
    let id = requested.lock().unwrap().id.clone();
    release.wait();
    ahead.join().unwrap();
    assert_eq!(
        *published.lock().unwrap(),
        None,
        "no stage started after the request"
    );
    assert!(
        current(&sessions, &id).is_some(),
        "the request's session stays"
    );
}

/// A synthetic paint shader: the vertex input `input` beside position and texture coordinate,
/// a constant colour and the blend `blend`.
fn paint_shader(input: &str, blend: &str) -> String {
    format!(
        r#"VertexStruct Input {{ float3 position : POSITION;
float2 uv : TEXCOORD0;
{input}
}};
VertexStruct Output {{ float4 position : SV_POSITION;
float2 uv : TEXCOORD0;
}};
ConstantBuffer(0,0) {{ float4x4 M;
float4 Color;
}};
VertexShader = {{ MainCode V [[
Output main(Input i) {{ Output o; o.position = mul(M, float4(i.position, 1)); o.uv = i.uv; return o; }}
]] }}
PixelShader = {{ MainCode P [[
float4 main(Output i) : SV_TARGET {{ return float4(0.25, 0.5, 0.75, 0.5); }}
]] }}
BlendState BlendState {{ BlendEnable = yes {blend} }}
Effect Up {{ VertexShader = V PixelShader = P }}
"#
    )
}

const SOURCE_OVER: &str = r#"SourceBlend = "SRC_ALPHA" DestBlend = "INV_SRC_ALPHA""#;

/// A window a content folder `name` declares, its icon drawn with `shader`.
fn paint_folder(name: &str, shader: &str) -> std::path::PathBuf {
    folder(
        name,
        &[
            (
                "interface/synthetic_paint.gui",
                br#"guiTypes = { containerWindowType = { name = "synthetic_paint_window" size = { width = 20 height = 20 }
                    iconType = { name = "mark" spriteType = "GFX_synthetic_paint" position = { x = 2 y = 3 } } } }"#
                    .to_vec(),
            ),
            (
                "interface/synthetic_paint.gfx",
                br#"spriteTypes = { spriteType = { name = "GFX_synthetic_paint" texturefile = "gfx/interface/synthetic_paint.tga" effectFile = "gfx/FX/synthetic_paint.lua" } }"#
                    .to_vec(),
            ),
            ("gfx/interface/synthetic_paint.tga", tga(8, 8)),
            ("gfx/FX/synthetic_paint.shader", shader.as_bytes().to_vec()),
        ],
    )
}

/// The synthetic window over `layers`, painted `paints` times by one session's painter, each
/// with what it drew from the recording instead of the visual mods.
fn painted_window(
    layers: &installed_content_core::ResolvedLayers,
    paints: usize,
) -> Vec<(crate::paint::Painted, std::collections::BTreeSet<String>)> {
    let localisation = installed_content_localisation::localisation::read(layers, "english").unwrap();
    let sessions = std::sync::Mutex::new(crate::session::Sessions::default());
    let session = crate::session::session(
        &sessions,
        "paint",
        layers,
        "english",
        installed_content_image::native::Decode::default(),
        [1920, 1080],
        crate::session::Opener::Request,
        &|| true,
    )
    .unwrap()
    .unwrap();
    let mut session = session.lock().unwrap();
    let session = &mut *session;
    let root = session
        .guis
        .template(b"synthetic_paint_window")
        .unwrap()
        .clone();
    let mut assets = viewer_foundation::map::native_gui::Assets::default();
    {
        let guis = &session.guis;
        assets.extend(viewer_foundation::map::native_gui::assets_in(
            &root,
            &|name: &[u8]| guis.template(name),
            &localisation,
        ));
    }
    session.load(layers, &assets).unwrap();
    let resources = crate::resources::WindowResources {
        gui: &session.gui,
        guis: &session.guis,
        localisation: &localisation,
    };
    let instance = compose::construct(&root, session.viewport, &resources).unwrap();
    let layout = instance
        .layout_with_regions(&root, &resources, None, true)
        .unwrap()
        .unwrap();
    (0..paints)
        .map(|_| {
            let (painted, fallbacks) = installed_content_core::content::observe_overlay_refusals(|| {
                crate::paint::paint(
                    &mut session.painter,
                    &session.images,
                    &session.gui,
                    layout.composed_parts(&vec![Vec::new(); layout.holders.len()]),
                    layout.size,
                )
                .unwrap()
            });
            (painted, fallbacks)
        })
        .collect()
}

/// A window a mod declares paints offscreen as the game draws it: its icon runs the mod's own
/// effect with its authored blend, and the image is straight alpha, clear where nothing draws.
#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn a_window_paints_its_parts_through_their_own_effects() {
    tables();
    let synthetic = paint_folder("paint", &paint_shader("", SOURCE_OVER));
    let mut painted = painted_window(&installed(&[&synthetic]), 1);
    let (painted, _) = painted.remove(0);
    assert!(painted.refused.lines().is_empty());
    let image = painted.image;
    assert_eq!([image.width, image.height], [20, 20]);
    let pixel = |x: u32, y: u32| {
        let at = ((y * 20 + x) * 4) as usize;
        <[u8; 4]>::try_from(&image.pixels[at..at + 4]).unwrap()
    };
    assert_eq!(pixel(0, 0), [0; 4]);
    let drawn = pixel(5, 6);
    assert!(
        drawn
            .iter()
            .zip([64, 128, 191, 128])
            .all(|(a, b)| a.abs_diff(b) <= 1),
        "{drawn:?}"
    );
}

/// A part whose colour blend cannot be flattened into an image composited later (additive over
/// transparent pixels) is left out by its effect's name, not drawn as something else.
#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn a_part_that_cannot_be_flattened_is_refused_by_name() {
    tables();
    let additive = paint_shader("", r#"SourceBlend = "ONE" DestBlend = "ONE""#);
    let synthetic = paint_folder("paint_additive", &additive);
    let (painted, _) = painted_window(&installed(&[&synthetic]), 1).remove(0);
    let refused = painted.refused.lines();
    assert!(
        refused
            .iter()
            .any(|why| why.contains("synthetic_paint") && why.contains("blends colour")),
        "{refused:?}"
    );
    assert!(painted.image.pixels.iter().all(|byte| *byte == 0));
}

/// A visual mod's replacement whose pipeline cannot be built draws the recording's effect, and
/// every paint names it: a later identical paint reuses the prepared draws, so the name comes
/// from what the painter kept.
#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn a_paint_time_fallback_is_named_on_every_paint() {
    use installed_content_core::content::visual;
    tables();
    let recorded = paint_folder("paint_recorded", &paint_shader("", SOURCE_OVER));
    // The same program writing whole numbers, which compiles but no float target accepts:
    // its pipeline cannot be built.
    let broken = paint_shader("", SOURCE_OVER).replace(
        "float4 main(Output i) : SV_TARGET { return float4(0.25, 0.5, 0.75, 0.5); }",
        "uint4 main(Output i) : SV_TARGET { return uint4(1, 2, 3, 4); }",
    );
    let art = folder(
        "paint_art",
        &[("gfx/FX/synthetic_paint.shader", broken.into_bytes())],
    );
    let user = folder(
        "paint_user",
        &[(
            "mod/art.mod",
            format!(
                "name=\"Art\"\npath=\"{}\"\n",
                art.display().to_string().replace('\\', "/")
            )
            .into_bytes(),
        )],
    );
    let mut layers = installed(&[&recorded]);
    let options = installed_content_core::ResolveOptions {
        game_dir: user.clone(),
        user_dir: user.clone(),
        cache_dir: user.join("cache"),
    };
    let choice = visual::Choice {
        id: "mod/art.mod".into(),
        name: "Art".into(),
    };
    let plan = visual::apply(&mut layers, &options, &[choice], &[], &Default::default()).unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let mut paints = painted_window(&layers, 2);
    let images: Vec<_> = paints
        .iter()
        .map(|(painted, _)| painted.image.clone())
        .collect();
    for (index, (painted, fallbacks)) in paints.drain(..).enumerate() {
        let refused = painted.refused.lines();
        assert!(refused.is_empty(), "paint {index}: {refused:?}");
        assert!(
            fallbacks
                .iter()
                .any(|fallback| fallback.starts_with("shader gfx/FX/synthetic_paint")),
            "paint {index}: {fallbacks:?}"
        );
    }
    // Both draw the recording's effect.
    assert_eq!(images[0].pixels, images[1].pixels);
}
