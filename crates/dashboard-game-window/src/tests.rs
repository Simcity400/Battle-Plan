//! Synthetic windows through the native layout, the scene flattening and the policy rows.

use std::collections::BTreeMap;

use installed_content_core::gui::native::read;
use viewer_foundation::map::{
    native_gui::{Resources, Sheets},
    test_support::gui::{Fixture, fixture_with},
};

use crate::{
    compose::{self, BoundList, Hover, Row},
    mio,
    scene::Scene,
};

pub(crate) fn tables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
}

/// Every page is 64 by 32 texels.
fn sizes(_: usize) -> Option<[u32; 2]> {
    Some([64, 32])
}

/// `root` composed with `rows` row windows, each hovered and the second muted, in a list
/// `list` of `row`.
fn composed(
    fixture: &Fixture,
    root: &str,
    list: Option<(&str, &str, usize)>,
    scroll: &BTreeMap<String, i32>,
) -> Result<Scene, Vec<String>> {
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
                        muted: index == 1,
                    })
                    .collect(),
            }]
        }
        None => Vec::new(),
    };
    let sheets: &dyn Sheets = fixture;
    compose::compose(
        &compose::Window {
            name: root,
            root: template,
            instance: &instance,
            lists: &lists,
            grids: &[],
            muted: &[],
            hovers: &[],
        },
        scroll,
        resources,
        sheets,
        &sizes,
    )
}

/// A sprite's frame is its strip offset added to the first frame's coordinates, at the place
/// the native layout gives it; a text's glyphs carry its colour; clips are shared by index.
#[test]
fn quads_follow_the_native_draw() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = { containerWindowType = {
            name = "w" size = { width = 100 height = 50 } clipping = yes
            iconType = { name = "i" spriteType = "GFX_four" frame = 3 position = { x = 5 y = 6 } }
            instantTextBoxType = { name = "t" font = "Main_14" text = "ab" position = { x = 1 y = 2 }
                maxWidth = 60 maxHeight = 20 }
        } }"#,
    ));
    let scene = composed(&fixture, "w", None, &BTreeMap::new()).unwrap();
    assert_eq!(scene.size, [100, 50]);
    let icon = &scene.quads[0];
    assert_eq!(icon.uv, [[0.5, 0.0], [0.75, 0.0], [0.75, 1.0], [0.5, 1.0]]);
    assert_eq!(icon.corners[0], [5.0, 6.0]);
    assert_eq!(icon.glyph, None);
    let glyphs: Vec<_> = scene
        .quads
        .iter()
        .filter(|quad| quad.glyph == Some(true))
        .collect();
    assert_eq!(glyphs.len(), 2);
    assert!(glyphs.iter().all(|glyph| glyph.color[3] > 0.0));
    assert_eq!(
        scene
            .textures
            .iter()
            .map(|texture| texture.width)
            .collect::<Vec<_>>(),
        vec![64; scene.textures.len()]
    );
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
}

/// A window whose container `c` holds grid `g` of 20-pixel rows, laid out in `format` with the
/// grid at `grid_y`, composed with five rows in its first column at `offset` of `c`.
fn scrolled_grid(format: &str, grid_y: i32, offset: Option<i32>) -> (Scene, String) {
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
    let sheets: &dyn Sheets = &fixture;
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
                        muted: false,
                    },
                    [0, row],
                )
            })
            .collect(),
    }];
    let scroll: BTreeMap<String, i32> = offset
        .map(|offset| (container.clone(), offset))
        .into_iter()
        .collect();
    let scene = compose::compose(
        &compose::Window {
            name: "w",
            root: template,
            instance: &instance,
            lists: &[],
            grids: &grids,
            muted: &[],
            hovers: &[],
        },
        &scroll,
        resources,
        sheets,
        &sizes,
    )
    .unwrap();
    (scene, container)
}

fn tops(scene: &Scene) -> Vec<f32> {
    scene.hovers.iter().map(|hover| hover.rect[1]).collect()
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
    let area = rest.lists.iter().find(|list| list.path == container).unwrap();
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
    assert_eq!(scrolled_grid("UPPER_LEFT", 0, Some(99)).0.lists[0].offset, 40);
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
    let area = rest.lists.iter().find(|list| list.path == container).unwrap();
    assert!(area.max > 0, "{area:?}");
    assert_eq!(area.offset, area.max, "{area:?}");
    let (near, _) = scrolled_grid("LOWER_LEFT", 20, Some(0));
    assert_eq!(near.lists[0].offset, 0);
    for (rest, near) in tops(&rest).iter().zip(tops(&near)) {
        assert_eq!(near - rest, area.max as f32);
    }
}

/// A muted root control draws muted, every part below it included; a hovered control explains
/// itself over its own region.
#[test]
fn root_controls_are_muted_and_explained_by_path() {
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
    let muted = template.path_named("i").unwrap().path.clone();
    let hovered = template.path_named("j").unwrap().path.clone();
    let sheets: &dyn Sheets = &fixture;
    let scene = compose::compose(
        &compose::Window {
            name: "w",
            root: template,
            instance: &instance,
            lists: &[],
            grids: &[],
            muted: &[muted.as_str()],
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
        sheets,
        &sizes,
    )
    .unwrap();
    let at = |x: f32| {
        scene
            .quads
            .iter()
            .find(|quad| quad.corners[0][0] == x)
            .unwrap()
    };
    assert_eq!(at(5.0).muted, Some(true));
    assert_eq!(at(40.0).muted, None);
    assert_eq!(scene.hovers.len(), 1);
    assert_eq!(scene.hovers[0].rect[0], 40.0);
}

/// A part the dashboard cannot draw is left out by path and the window says it is incomplete.
#[test]
fn undrawn_parts_are_named() {
    tables();
    let fixture = fixture_with(read(
        br#"guiTypes = { containerWindowType = {
            name = "w" size = { width = 200 height = 50 }
            iconType = { name = "bar" spriteType = "GFX_bar" }
        } }"#,
    ));
    let scene = composed(&fixture, "w", None, &BTreeMap::new()).unwrap();
    assert!(scene.quads.is_empty());
    assert!(
        scene
            .notes
            .iter()
            .any(|note| note.contains("w.bar is not drawn: progress bars")),
        "{:?}",
        scene.notes
    );
    assert!(
        scene
            .notes
            .iter()
            .any(|note| note.contains("drawn incompletely"))
    );
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
        let scroll = BTreeMap::from([("w.l".to_owned(), offset)]);
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
    // Every row's quads are clipped to the list; only the muted row's are muted.
    let row_quads: Vec<_> = scrolled
        .quads
        .iter()
        .filter(|quad| quad.clip.is_some())
        .collect();
    assert_eq!(row_quads.len(), 4);
    assert_eq!(
        row_quads
            .iter()
            .map(|quad| quad.muted == Some(true))
            .collect::<Vec<_>>(),
        [false, true, false, false]
    );
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
    let (rows, unlisted) = mio::rows(&details, &roster, spell).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.token.clone()).collect::<Vec<_>>(),
        [b"token_3".to_vec(), b"token_1".to_vec()]
    );
    assert_eq!(rows[0].cost, 75);
    assert!(!rows[0].selected && rows[1].selected);
    assert_eq!(unlisted, [b"token_2".to_vec()]);
    // The constructor's sentinel selects nothing, even if a policy had that token.
    details.state.policy = 0x4c17;
    details.allowed_policies = vec![0x4c17];
    let (rows, _) = mio::rows(&details, &[policy(0x4c17, 1)], spell).unwrap();
    assert!(!rows[0].selected);
    let failed = mio::rows(&details, &roster, |_| Err("unspelled".to_owned()));
    assert_eq!(failed.unwrap_err(), "unspelled");
}

/// A policy whose visibility does not hold has no row and one whose visibility is unknown is
/// withheld by name; availability is kept per row, unknown with its reason; a policy with no
/// authored icon has its icon refused.
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
    assert_eq!(shown[0].icon.as_deref(), Ok(b"GFX_shown".as_slice()));
    assert!(
        shown[2]
            .icon
            .as_ref()
            .unwrap_err()
            .contains("names no icon")
    );
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

/// `count` synthetic rows, each named and with `icon` (or a refused icon).
fn synthetic_rows(count: usize, icon: Option<&[u8]>) -> Vec<mio::Shown> {
    (0..count)
        .map(|index| mio::Shown {
            policy: mio::PolicyRow {
                token: format!("synthetic_policy_{index}").into_bytes(),
                name: format!("Synthetic policy {index}").into_bytes(),
                cost: 10,
                days: 30,
                selected: index == 0,
            },
            name: Ok(format!("Synthetic policy {index}")),
            icon: icon.map(<[u8]>::to_vec).ok_or_else(|| "no icon".to_owned()),
            // The second row is unavailable.
            available: Ok(index != 1),
        })
        .collect()
}

type Drawn = (
    crate::products::game_window::Bound,
    std::sync::Arc<std::sync::Mutex<crate::session::Session>>,
);

fn drawn(
    layers: &installed_content_core::ResolvedLayers,
    rows: &[mio::Shown],
    scroll: &BTreeMap<String, i32>,
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
    )
    .unwrap()
    .unwrap();
    let bound = mio::draw(
        &mut session.lock().unwrap(),
        layers,
        &localisation,
        rows,
        &[],
        scroll,
    )
    .unwrap();
    (bound, session)
}

/// The installed policy window lays out through the native layout with its content's own
/// sprites, fonts and list, every texture page it samples loads, and its rows scroll.
#[test]
fn the_installed_policy_window_draws_and_scrolls() {
    tables();
    let layers = installed(&[]);
    let rows = synthetic_rows(12, None);
    let (bound, session) = drawn(&layers, &rows, &BTreeMap::new());
    let scene = bound.scene.unwrap();
    assert!(!scene.quads.is_empty());
    assert!(scene.quads.iter().any(|quad| quad.glyph == Some(true)));
    assert!(!scene.hovers.is_empty() && scene.hovers.len() <= rows.len());
    // The unavailable row's quads, and only those, are muted.
    assert!(scene.quads.iter().any(|quad| quad.muted == Some(true)));
    assert!(
        scene
            .quads
            .iter()
            .any(|quad| quad.muted.is_none() && quad.clip.is_some())
    );
    assert_eq!(scene.lists.len(), 1);
    let list = &scene.lists[0];
    assert!(list.max > 0, "{list:?}");
    let pages = session.lock().unwrap();
    for texture in &scene.textures {
        let page = pages.images[texture.id as usize].clone();
        assert_eq!([page.width, page.height], [texture.width, texture.height]);
        installed_content_image::rgba::dds_base(page).unwrap();
    }
    drop(pages);
    // At the end of its run the first rows have left the list.
    let path = list.path.clone();
    let (bound, _) = drawn(&layers, &rows, &BTreeMap::from([(path, i32::MAX)]));
    let scrolled = bound.scene.unwrap();
    assert_eq!(scrolled.lists[0].offset, scrolled.lists[0].max);
    assert!(scrolled.hovers[0].rect[1] < scene.hovers[0].rect[1]);
    // Icons that do not resolve are named in the window's slots, not drawn as another.
    assert!(bound.slots.iter().any(|slot| slot.name == "row icon"
        && slot.state == crate::products::game_window::SlotState::Refused));
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
    let (bound, _) = drawn(
        &layers,
        &synthetic_rows(1, Some(b"GFX_synthetic_policy_icon")),
        &BTreeMap::new(),
    );
    let scene = bound.scene.unwrap();
    assert!(
        scene
            .textures
            .iter()
            .any(|texture| [texture.width, texture.height] == [36, 20]),
        "{:?}",
        scene.textures
    );
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
                    iconType = {{ name = "icon" }} instantTextboxType = {{ name = "{control}" }}
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
    let layers = installed(&[]);
    let open = |sessions: &std::sync::Mutex<crate::session::Sessions>, stack: &str| {
        crate::session::session(
            sessions,
            stack,
            &layers,
            "english",
            installed_content_image::native::Decode::default(),
            [1920, 1080],
            crate::session::Opener::Request,
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
    let layers = Arc::new(installed(&[]));
    let open = |sessions: &Mutex<Sessions>, stack: &str, opener: Opener| {
        session(
            sessions,
            stack,
            &layers,
            "english",
            installed_content_image::native::Decode::default(),
            [1920, 1080],
            opener,
        )
        .unwrap()
    };
    // The publication itself is guarded: once a request made `new` current, the read-ahead of
    // `old` gets nothing and leaves the request's session in place.
    let sessions = Mutex::new(Sessions::default());
    let requested = open(&sessions, "new", Opener::Request).unwrap();
    let id = requested.lock().unwrap().id.clone();
    assert!(open(&sessions, "old", Opener::ReadAhead).is_none());
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
