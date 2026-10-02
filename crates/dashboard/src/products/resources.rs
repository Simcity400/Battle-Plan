//! Strategic resources as every resource product shows them: the recorded
//! `ResourceDefinitions` in the game's order, told apart by native index, with
//! the names and icons the recorded content stack gives them.
//!
//! Shared by Resource balance and Trade flows, so a resource is named, pictured
//! and numbered the same way everywhere, and its icons are decoded once per
//! catalogue whichever product asks first.

use super::LoadContext;
use crate::{
    Source,
    catalogue::{self, Catalogue},
};
use installed_content::{ResolvedLayers, gfx::SpriteSet, localisation::Localisation};
use replay::rows::{LexicalTokenRow, ResourceDefinitionRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

/// Decimal places of every recorded resource amount.
pub const RESOURCE_DECIMALS: u8 = 5;
/// Longest side of a returned resource icon.
const ICON_EDGE: u32 = 48;
/// The trade view's resource row and its icon button, the element the game gives
/// each resource's `icon_frame`: `CTradeResourceInfoItem::CTradeResourceInfoItem@141CA4B50`
/// opens template literal 142A4A878 and asks it for button literal 1429C0690
/// (FindButton, vtable +0x68). Vanilla declares the row in
/// `interface/countrytradeview.gui`; any GUI file may declare it.
const RESOURCE_ROW: &str = "resources_info_entry";
const RESOURCE_ICON: &str = "resources_icon";

/// A recorded resource, as content names and pictures it. The game keeps every
/// declaration of a resource as its own resource, even one repeating a token
/// (`CStrategicResourceDatabase::load_file@140AB2ED0` interns each declared key and
/// appends a new item and key with no check for an earlier one), so resources are
/// told apart by their native index and a repeated token is numbered.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// The native index (`ResourceDefinitions.id`).
    pub id: u32,
    pub token: String,
    /// Which declaration of its token this is, from 1, in the game's order.
    pub ordinal: u32,
    pub name: Option<String>,
    /// A PNG data URL; null when content has no icon for it.
    pub icon: Option<String>,
}

/// The recorded resources seen so far, by native index, in the game's order.
#[derive(Clone, Debug, Default)]
pub struct Learned {
    ids: BTreeMap<u32, String>,
}

impl Learned {
    /// Adds the resources a frame's definitions name that were not seen yet. A
    /// definition whose token the recorded dictionary does not spell is left for
    /// a later frame.
    pub fn learn(&mut self, definitions: &[ResourceDefinitionRow], lexical: &[LexicalTokenRow]) {
        for definition in definitions {
            if self.ids.contains_key(&definition.id) {
                continue;
            }
            let Some(token) = lexical
                .binary_search_by_key(&definition.token, |row| row.token)
                .ok()
                .and_then(|index| std::str::from_utf8(&lexical[index].literal).ok())
                .filter(|token| !token.is_empty())
            else {
                continue;
            };
            self.ids.insert(definition.id, token.to_owned());
        }
    }

    pub fn contains(&self, id: u32) -> bool {
        self.ids.contains_key(&id)
    }

    /// Native index and token of every resource seen, in the game's order.
    pub fn list(&self) -> Vec<(u32, String)> {
        self.ids
            .iter()
            .map(|(id, token)| (*id, token.clone()))
            .collect()
    }
}

/// Which declaration of its token each resource is, from 1, in the game's order:
/// the game appends every declaration, so the k-th native resource with a token is
/// its k-th declaration in load order.
pub fn ordinals(resources: &[(u32, String)]) -> Vec<u32> {
    let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
    resources
        .iter()
        .map(|(_, token)| {
            let count = seen.entry(token.as_str()).or_default();
            *count += 1;
            *count
        })
        .collect()
}

/// Names and icons, and the standing warnings, for the catalogue they were read
/// from; one per source, whichever resource product asks.
#[derive(Default)]
struct ContentState {
    content: Option<ContentCache>,
}

struct ContentCache {
    catalogue: Arc<Catalogue>,
    /// By token and which declaration of it (from 1).
    resources: BTreeMap<(String, u32), Declared>,
    warnings: Vec<String>,
}

/// A declaration's name and icon from content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Declared {
    pub name: Option<String>,
    pub icon: Option<String>,
}

/// What resource names and icons are read from: one catalogue's layers, sprites and
/// localisation, all from the recorded content stack.
pub(crate) struct ContentView<'a> {
    pub layers: &'a ResolvedLayers,
    pub sprites: &'a SpriteSet,
    pub localisation: &'a Localisation,
    /// The game's GUI registry and texture decode settings, as the catalogue loaded them.
    pub guis: &'a installed_content::gui::native::Guis,
    pub gui_decode: &'a Result<installed_content::image::native::Decode, String>,
}

impl<'a> ContentView<'a> {
    pub(crate) fn of(catalogue: &'a Catalogue) -> Self {
        Self {
            layers: &catalogue.content.layers,
            sprites: &catalogue.sprites,
            localisation: &catalogue.localisation,
            guis: &catalogue.guis,
            gui_decode: &catalogue.gui_decode,
        }
    }
}

/// The icon sprite of the trade view's resource row, and each content declaration's
/// frame in it: per token, its declarations in load order.
struct Icons {
    texture: String,
    frames: u32,
    icon_frames: BTreeMap<String, Vec<Option<u32>>>,
}

fn icons(view: &ContentView) -> io::Result<(Icons, Vec<String>)> {
    let mut warnings = Vec::new();
    let name = installed_content::gui::window_button_sprite(
        view.guis,
        RESOURCE_ROW,
        RESOURCE_ICON,
        &mut warnings,
    )?;
    let sprite = catalogue::created_gui_sprite(view.sprites, &name)?;
    let texture = sprite
        .texture
        .clone()
        .ok_or_else(|| crate::invalid(format!("{name} has no texture")))?;
    let (resources, notes) = installed_content::resources::read(view.layers)?;
    warnings.extend(notes);
    let mut icon_frames: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
    for resource in resources {
        icon_frames
            .entry(resource.token)
            .or_default()
            .push(resource.icon_frame);
    }
    Ok((
        Icons {
            texture,
            frames: sprite.frames,
            icon_frames,
        },
        warnings,
    ))
}

/// A resource's name as the game shows it. GetLocalizedResourceName
/// (140BBC360), which the trade view (1415EA940), the state resource tooltip
/// (141DADF30) and the other resource texts call, localises
/// `PRODUCTION_MATERIALS_` (14293AF98) followed by the token upper-cased by the
/// C runtime's toupper (1424B87F0 -> 14254F1FC), through the plain localiser
/// (142230F20) only: brackets are glyphs ([`catalogue::View::Plain`]).
/// `None`, with the reason in `warnings`, when there is no name to show.
fn resource_name(
    localisation: &Localisation,
    token: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let upper = installed_content::localisation::crt_case_text(token, true, false);
    let key = format!("PRODUCTION_MATERIALS_{upper}");
    // An empty text is shown empty, as the game's text box shows it; a key
    // the content lacks is its own bytes (142230940).
    catalogue::localised(localisation, &key, catalogue::View::Plain)
        .inspect_err(|reason| warnings.push(format!("{token}: no name: {reason}")))
        .ok()
}

/// Names and icons for `wanted` declarations (token, ordinal) from content: the name
/// the game's resource texts use ([`resource_name`]) and the declaration's
/// frame of the trade view's icon. A missing key shows as that key; a refused name
/// or missing icon stays unavailable, with a warning.
pub(crate) fn resource_content(
    view: &ContentView,
    wanted: &[(String, u32)],
    cached: &mut BTreeMap<(String, u32), Declared>,
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let missing: Vec<&(String, u32)> = wanted
        .iter()
        .filter(|key| !cached.contains_key(*key))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let icons = match icons(view) {
        Ok((icons, notes)) => {
            warnings.extend(notes);
            Some(icons)
        }
        Err(error) => {
            warnings.push(format!("Resource icons are unavailable: {error}"));
            None
        }
    };
    for (token, ordinal) in missing {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Resource request cancelled",
            ));
        }
        let mut notes = Vec::new();
        let name = resource_name(view.localisation, token, &mut notes);
        if *ordinal == 1 {
            warnings.extend(notes);
        }
        let icon = match icons.as_ref().map(|icons| {
            let frame = icons
                .icon_frames
                .get(token.as_str())
                .and_then(|frames| frames.get(*ordinal as usize - 1))
                .copied()
                .flatten();
            (icons, frame)
        }) {
            Some((icons, Some(frame))) => match view
                .gui_decode
                .as_ref()
                .map_err(crate::invalid)
                .and_then(|decode| {
                    catalogue::gui_texture_frame_png(
                        view.layers,
                        icons.texture.as_bytes(),
                        *decode,
                        icons.frames,
                        frame - 1,
                        ICON_EDGE,
                    )
                }) {
                Ok(icon) => Some(icon),
                Err(error) => {
                    warnings.push(format!("{token}: icon unavailable: {error}"));
                    None
                }
            },
            Some((_, None)) => {
                warnings.push(format!(
                    "{token}: the recorded content gives no usable icon frame."
                ));
                None
            }
            None => None,
        };
        cached.insert((token.clone(), *ordinal), Declared { name, icon });
    }
    Ok(())
}

/// Every recorded resource (native index and token, in the game's order) with its
/// content name and icon, and the warnings that go with them. Without the game
/// folders each keeps its token and has no icon, and a warning says why.
pub fn resource_infos(
    source: &Source,
    context: &LoadContext,
    resources: &[(u32, String)],
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ResourceInfo>, Vec<String>)> {
    let mut warnings = Vec::new();
    let ordinals = ordinals(resources);
    let declarations: Vec<(String, u32)> = resources
        .iter()
        .zip(&ordinals)
        .map(|((_, token), ordinal)| (token.clone(), *ordinal))
        .collect();
    let info = |index: usize, declared: Option<&Declared>| {
        let (id, token) = &resources[index];
        ResourceInfo {
            id: *id,
            token: token.clone(),
            ordinal: ordinals[index],
            name: declared.and_then(|declared| declared.name.clone()),
            icon: declared.and_then(|declared| declared.icon.clone()),
        }
    };
    let infos = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            let state = source.state::<ContentState>();
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            if !cache
                .content
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(&cached.catalogue, &catalogue))
            {
                let mut standing = catalogue.verification.clone();
                standing.extend(catalogue.warnings.iter().cloned());
                cache.content = Some(ContentCache {
                    catalogue: Arc::clone(&catalogue),
                    resources: BTreeMap::new(),
                    warnings: standing,
                });
            }
            let content = cache.content.as_mut().expect("content cached");
            resource_content(
                &ContentView::of(&catalogue),
                &declarations,
                &mut content.resources,
                &mut content.warnings,
                cancelled,
            )?;
            warnings.extend(content.warnings.iter().cloned());
            let infos: Vec<ResourceInfo> = declarations
                .iter()
                .enumerate()
                .map(|(index, key)| info(index, content.resources.get(key)))
                .collect();
            if infos.iter().any(|resource| resource.icon.is_some()) {
                warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
            }
            infos
        }
        Err(error) => {
            warnings.push(format!("Resource names and icons are unavailable: {error}"));
            (0..resources.len())
                .map(|index| info(index, None))
                .collect()
        }
    };
    // Each declaration is its own resource in the game, so each is shown, numbered.
    for (index, (_, token)) in resources.iter().enumerate() {
        if ordinals[index] == 2 {
            let count = resources.iter().filter(|(_, other)| other == token).count();
            warnings.push(format!(
                "{token} is declared {count} times; the game keeps each declaration as its own resource, so each is shown and numbered."
            ));
        }
    }
    Ok((infos, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::{fs, path::Path};

    /// A true-colour Targa strip, one solid colour per frame, each frame 2 × 2.
    fn strip(colours: &[[u8; 3]]) -> Vec<u8> {
        let width = colours.len() as u16 * 2;
        let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        // 32 bits a pixel, top-left origin, 8 alpha bits.
        bytes.extend_from_slice(&[32, 0x28]);
        for _ in 0..2 {
            for [red, green, blue] in colours {
                for _ in 0..2 {
                    bytes.extend_from_slice(&[*blue, *green, *red, 255]);
                }
            }
        }
        bytes
    }

    fn write(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// The first pixel of a PNG data URL.
    fn first_pixel(url: &str) -> [u8; 3] {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        [pixels[0], pixels[1], pixels[2]]
    }

    const RED: [u8; 3] = [200, 0, 0];
    const GREEN: [u8; 3] = [0, 200, 0];
    const BLUE: [u8; 3] = [0, 0, 200];

    /// Vanilla-like content (oil and steel, the trade view in `countrytradeview.gui`)
    /// and a mod adding unobtainium and its own trade view in a file of its own, with
    /// a three-frame icon strip; `replace` makes the mod replace `interface`.
    fn modded(root: &Path, replace: bool) -> ResolvedLayers {
        let _ = fs::remove_dir_all(root);
        let (base, add) = (root.join("base"), root.join("mod"));
        write(
            &base,
            "interface/countrytradeview.gui",
            br#"guiTypes = { containerWindowType = { name = "resources_info_entry" buttonType = { name = "resources_icon" spriteType = "GFX_base_strip" } } }"#,
        );
        write(
            &base,
            "interface/base.gfx",
            br#"spriteTypes = { spriteType = { name = "GFX_base_strip" texturefile = "gfx/interface/base.tga" noOfFrames = 2 } }"#,
        );
        write(&base, "gfx/interface/base.tga", &strip(&[GREEN, GREEN]));
        write(
            &base,
            "common/resources/00_resources.txt",
            b"resources = { oil = { icon_frame = 1 } steel = { icon_frame = 2 } }",
        );
        write(
            &add,
            "interface/zz_mod_trade.gui",
            br#"guiTypes = { containerWindowType = { name = "resources_info_entry" buttonType = { name = "resources_icon" spriteType = "GFX_mod_strip" } } }"#,
        );
        write(
            &add,
            "interface/zz_mod.gfx",
            br#"spriteTypes = { spriteType = { name = "GFX_mod_strip" texturefile = "gfx/interface/mod_strip.tga" noOfFrames = 3 } }"#,
        );
        write(
            &add,
            "gfx/interface/mod_strip.tga",
            &strip(&[RED, GREEN, BLUE]),
        );
        // A file the game's parser cannot finish; the others are still read.
        write(&add, "interface/broken.gui", b"guiTypes = { \"unterminated");
        write(
            &add,
            "common/resources/mod_resources.txt",
            b"resources = { unobtainium = { icon_frame = 3 } }",
        );
        let replaced: &[&str] = if replace { &["interface"] } else { &[] };
        ResolvedLayers::from_folders(&[(&base, &[][..]), (&add, replaced)]).unwrap()
    }

    fn declared(layers: &ResolvedLayers) -> (Vec<Declared>, Vec<String>) {
        let sprites = installed_content::gfx::read(layers).unwrap();
        let localisation = crate::catalogue::tests::loaded(concat!(
            "l_english:\n",
            " PRODUCTION_MATERIALS_OIL:0 \"Oil\"\n",
            " PRODUCTION_MATERIALS_UNOBTAINIUM:0 \"Unobtainium\"\n",
        ));
        let guis = installed_content::gui::native::read_layers(layers).unwrap();
        let view = ContentView {
            layers,
            sprites: &sprites,
            localisation: &localisation,
            guis: &guis,
            gui_decode: &Ok(Default::default()),
        };
        let wanted: Vec<(String, u32)> = ["oil", "steel", "unobtainium"]
            .iter()
            .map(|token| (token.to_string(), 1))
            .collect();
        let (mut cached, mut warnings) = (BTreeMap::new(), Vec::new());
        resource_content(
            &view,
            &wanted,
            &mut cached,
            &mut warnings,
            &AtomicBool::new(false),
        )
        .unwrap();
        (
            wanted.iter().map(|key| cached[key].clone()).collect(),
            warnings,
        )
    }

    #[test]
    fn a_mod_resource_takes_its_name_and_icon_from_the_mod_s_own_trade_view() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_modded_resources_{}",
            std::process::id()
        ));
        // A total conversion: the mod replaces `interface`, and its trade view lives
        // in a file of its own name.
        let (resources, warnings) = declared(&modded(&root, true));
        let names: Vec<_> = resources.iter().map(|item| item.name.as_deref()).collect();
        // A missing localisation key shows as the game shows it.
        assert_eq!(
            names,
            [
                Some("Oil"),
                Some("PRODUCTION_MATERIALS_STEEL"),
                Some("Unobtainium")
            ]
        );
        let pixels: Vec<_> = resources
            .iter()
            .map(|item| first_pixel(item.icon.as_deref().unwrap()))
            .collect();
        // Each declaration's own frame of the mod's strip.
        assert_eq!(pixels, [RED, GREEN, BLUE]);
        // A file the game's parser cannot finish does not hide the rest.
        assert!(
            warnings.iter().all(|warning| !warning.contains("icon")),
            "{warnings:?}"
        );
        // An addition beside vanilla's trade view: both files register the row, and the
        // registry keeps the last in the engine's load order (`CGui::FindGuiType`).
        let (resources, _) = declared(&modded(&root, false));
        assert_eq!(first_pixel(resources[2].icon.as_deref().unwrap()), BLUE);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_decode_settings_keep_resource_names_and_refuse_icons_by_token() {
        let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "interface/test.gui",
                "guiTypes = { containerWindowType = { name = resources_info_entry buttonType = { name = resources_icon spriteType = icon } } }",
            ),
            (
                "interface/test.gfx",
                "spriteTypes = { spriteType = { name = icon texturefile = gfx/icon.dds } }",
            ),
            (
                "common/resources/test.txt",
                "resources = { ore = { icon_frame = 1 } }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n PRODUCTION_MATERIALS_ORE:0 \"Ore\"\n",
            ),
        ]);
        catalogue.guis = installed_content::gui::native::read_layers(&catalogue.content.layers).unwrap();
        catalogue.gui_decode = Err("settings.txt is unavailable".into());
        let mut cached = Default::default();
        let mut warnings = Vec::new();
        resource_content(
            &ContentView::of(&catalogue),
            &[("ore".into(), 1)],
            &mut cached,
            &mut warnings,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let declared = &cached[&("ore".into(), 1)];
        assert_eq!(declared.name.as_deref(), Some("Ore"));
        assert!(declared.icon.is_none());
        assert!(
            warnings
                .iter()
                .any(|why| why.contains("ore: icon unavailable: settings.txt")),
            "{warnings:?}"
        );
    }
}

#[cfg(test)]
mod naming_tests {
    use super::resource_name;

    /// The name is `PRODUCTION_MATERIALS_` and the upper-cased token through
    /// the plain localiser (140BBC360 -> 142230F20), loaded as content loads.
    #[test]
    fn a_resource_is_named_as_the_games_resource_texts_name_it() {
        let localisation = crate::catalogue::tests::loaded(concat!(
            "l_english:\n",
            " PRODUCTION_MATERIALS_OIL:0 \"Oil [ROOT.GetName]\"\n",
            " PRODUCTION_MATERIALS_STEEL:0 \"\"\n",
            " PRODUCTION_MATERIALS_COAL:0 \"a¤b\"\n",
            " state_resource_oil:0 \"Not the name\"\n",
        ));
        let mut warnings = Vec::new();
        // Mixed case: toupper folds the token, so both name the same key.
        for token in ["oil", "Oil"] {
            assert_eq!(
                resource_name(&localisation, token, &mut warnings).as_deref(),
                Some("Oil [ROOT.GetName]")
            );
        }
        assert_eq!(
            resource_name(&localisation, "steel", &mut warnings).as_deref(),
            Some("")
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            resource_name(&localisation, "tungsten", &mut warnings).as_deref(),
            Some("PRODUCTION_MATERIALS_TUNGSTEN")
        );
        assert_eq!(resource_name(&localisation, "coal", &mut warnings), None);
        // The "C" locale's toupper leaves a byte above 0x7F.
        assert_eq!(
            resource_name(&localisation, "stål", &mut warnings).as_deref(),
            Some("PRODUCTION_MATERIALS_STåL")
        );
        assert_eq!(
            warnings,
            [
                "coal: no name: localisation PRODUCTION_MATERIALS_COAL holds the control byte 0x14 (an authored `¤`), which plain text cannot show",
            ]
        );
    }
}
