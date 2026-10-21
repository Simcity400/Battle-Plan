//! Strategic resources as every resource product shows them: the recorded
//! `ResourceDefinitions` in the game's order, told apart by native index, with
//! the names and icons the recorded content stack gives them.
//!
//! Shared by Resource balance and Trade flows, so a resource is named, pictured
//! and numbered the same way everywhere, and its icons are drawn once per
//! catalogue whichever product asks first.

use super::LoadContext;
use crate::{
    Source,
    catalogue::{self, Catalogue},
};
use installed_content_core::ResolvedLayers;
use installed_content_gfx::gfx::SpriteSet;
use installed_content_localisation::localisation::Localisation;
use replay_data::{rows::LexicalTokenRow, rows::ResourceDefinitionRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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
/// The trade view's resource row and its icon button, the element that receives
/// each resource's `icon_frame`. Any GUI file may declare the row.
const RESOURCE_ROW: &str = "resources_info_entry";
const RESOURCE_ICON: &str = "resources_icon";

/// A recorded resource, as content names and pictures it. Every declaration
/// adds its own resource, even when repeating a token. Resources are told
/// apart by their recorded index and a repeated token is numbered.
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Learned {
    ids: BTreeMap<u32, String>,
}

impl crate::prepared::prepass::Held for Learned {
    fn held(&self, bytes: &mut crate::prepared::prepass::HeldBytes) {
        bytes.heap(&self.ids);
    }
}

impl Learned {
    /// Adds the resources a frame's definitions name that were not seen yet. A
    /// definition whose token the recorded dictionary does not spell is left for
    /// a later frame.
    pub fn learn(
        &mut self,
        definitions: &[ResourceDefinitionRow],
        lexical: &[LexicalTokenRow],
    ) -> bool {
        let mut changed = false;
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
            changed = true;
        }
        changed
    }

    pub fn contains(&self, id: u32) -> bool {
        self.ids.contains_key(&id)
    }

    pub fn learn_names(&mut self, names: &Self) -> bool {
        let before = self.ids.len();
        for (&id, token) in &names.ids {
            self.ids.entry(id).or_insert_with(|| token.clone());
        }
        before != self.ids.len()
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
pub struct Declared {
    pub name: Option<String>,
    pub icon: Option<String>,
}

/// What resource names and icons are read from: one catalogue's layers, sprites and
/// localisation, all from the recorded content stack.
pub struct ContentView<'a> {
    pub layers: &'a ResolvedLayers,
    pub sprites: &'a SpriteSet,
    pub localisation: &'a Localisation,
    /// The GUI registry or its named read failure; resource names are independent.
    pub guis: Result<&'a installed_content_core::gui::native::Guis, String>,
    pub gui_decode: &'a Result<installed_content_image::native::Decode, String>,
}

impl<'a> ContentView<'a> {
    pub fn of(catalogue: &'a Catalogue) -> Self {
        Self {
            layers: &catalogue.content.layers,
            sprites: &catalogue.sprites,
            localisation: &catalogue.localisation,
            guis: catalogue.guis().map_err(|error| error.to_string()),
            gui_decode: &catalogue.gui_decode,
        }
    }
}

/// The icon sprite of the trade view's resource row, and each content declaration's
/// one-based frame in it: per token, its declarations in load order.
struct Icons {
    sprite: installed_content_gfx::gfx::SpriteType,
    icon_frames: BTreeMap<String, Vec<Option<u32>>>,
}

fn icons(view: &ContentView) -> io::Result<(Icons, Vec<String>)> {
    let mut warnings = Vec::new();
    let name = installed_content_core::gui::native::window_button_sprite(
        view.guis.as_ref().map_err(crate::invalid)?,
        RESOURCE_ROW,
        RESOURCE_ICON,
        &mut warnings,
    )?;
    let sprite = catalogue::created_gui_sprite(view.sprites, &name)?.clone();
    let (resources, notes) = installed_content_core::resources::read(view.layers)?;
    warnings.extend(notes.into_iter().map(|(_, message)| message));
    let mut icon_frames: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
    for resource in resources {
        icon_frames
            .entry(resource.token)
            .or_default()
            .push(resource.icon_frame);
    }
    Ok((
        Icons {
            sprite,
            icon_frames,
        },
        warnings,
    ))
}

/// A resource's name as the game shows it: `PRODUCTION_MATERIALS_` followed by
/// the upper-cased token, through the plain localiser only. Brackets are glyphs
/// ([`catalogue::View::Plain`]).
/// `None`, with the reason in `warnings`, when there is no name to show.
fn resource_name(
    localisation: &Localisation,
    token: &str,
    warnings: &mut Vec<String>,
    findings: &mut Vec<dashboard_core::findings::Finding>,
) -> Option<String> {
    let upper = installed_content_localisation::localisation::crt_case_text(token, true, false);
    let key = format!("PRODUCTION_MATERIALS_{upper}");
    // Recovery and text share the walk; missing keys are byte copies.
    let (name, notes) = catalogue::localised_details(
        localisation,
        &key,
        &[],
        catalogue::View::Plain,
        dashboard_core::findings::Kind::Unsupported,
    );
    warnings.extend(
        notes
            .iter()
            .filter(|finding| finding.kind != dashboard_core::findings::Kind::Content)
            .map(|finding| format!("{token}: no name: {}", finding.message)),
    );
    findings.extend(notes);
    name
}

/// Names and icons for `wanted` declarations (token, ordinal) from content: the name
/// the game's resource texts use ([`resource_name`]) and the declaration's
/// frame of the trade view's icon. A missing key shows as that key; a refused name
/// or missing icon stays unavailable, with a warning.
pub fn resource_content(
    view: &ContentView,
    wanted: &[(String, u32)],
    cached: &mut BTreeMap<(String, u32), Declared>,
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
    let missing: Vec<&(String, u32)> = wanted
        .iter()
        .filter(|key| !cached.contains_key(*key))
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    let mut findings = Vec::new();
    let icons = match icons(view) {
        Ok((icons, notes)) => {
            findings.extend(
                notes
                    .iter()
                    .map(|note| Finding::new(Kind::Warning, "resource icons", note)),
            );
            warnings.extend(notes);
            Some(icons)
        }
        Err(error) => {
            let message = format!("Resource icons are unavailable: {error}");
            findings.push(Finding::new(Kind::Refused, "resource icons", &message));
            warnings.push(message);
            None
        }
    };
    let decode = match (&icons, view.gui_decode) {
        (Some(_), Ok(decode)) => Some(*decode),
        (Some(_), Err(why)) => {
            let message = format!("Resource icons are unavailable: {why}");
            findings.push(Finding::new(Kind::Refused, "resource icons", &message));
            warnings.push(message);
            None
        }
        (None, _) => None,
    };
    let frame_of = |icons: &Icons, token: &str, ordinal: u32| {
        icons
            .icon_frames
            .get(token)
            .and_then(|frames| frames.get(ordinal as usize - 1))
            .copied()
            .flatten()
    };
    // Draw every wanted frame of the shared sprite in one batch. A refused
    // batch leaves each frame to its own draw below, which names the refusal.
    if let (Some(icons), Some(decode)) = (&icons, decode) {
        let images = view.layers.derived::<catalogue::SpriteImages>();
        let frames: BTreeSet<u32> = missing
            .iter()
            .filter_map(|(token, ordinal)| frame_of(icons, token, *ordinal))
            .map(|frame| frame - 1)
            .filter(|frame| {
                !images.contains(&catalogue::sprite_image_key(
                    &icons.sprite,
                    decode,
                    *frame,
                    ICON_EDGE,
                ))
            })
            .collect();
        if frames.len() > 1 {
            let requests: Vec<_> = frames.iter().map(|frame| (&icons.sprite, *frame)).collect();
            let _ = catalogue::sprite_frames_png(view.layers, &requests, decode, ICON_EDGE);
        }
    }
    // A sprite draws every frame or none: only the frame offset differs between
    // them, and the resolved program samples past the strip as the game does.
    let mut refused = false;
    for (token, ordinal) in missing {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Resource request cancelled",
            ));
        }
        let mut notes = Vec::new();
        let mut name_findings = Vec::new();
        let name = resource_name(view.localisation, token, &mut notes, &mut name_findings);
        for finding in &mut name_findings {
            finding.subject = format!("resource {token} declaration {ordinal}");
        }
        findings.extend(name_findings);
        if *ordinal == 1 {
            warnings.extend(notes);
        }
        let Some(icons) = &icons else {
            cached.insert((token.clone(), *ordinal), Declared { name, icon: None });
            continue;
        };
        let icon = match (
            frame_of(icons, token, *ordinal),
            decode.filter(|_| !refused),
        ) {
            (Some(frame), Some(decode)) => match catalogue::sprite_frame_png(
                view.layers,
                &icons.sprite,
                decode,
                frame - 1,
                ICON_EDGE,
            ) {
                Ok(icon) => Some(icon),
                Err(error) => {
                    refused = true;
                    let message = format!("Resource icons are unavailable: {error}");
                    findings.push(Finding::new(Kind::Refused, "resource icons", &message));
                    warnings.push(message);
                    None
                }
            },
            (Some(_), None) => None,
            (None, _) => {
                let message = format!("{token}: the recorded content gives no usable icon frame.");
                findings.push(Finding::new(
                    Kind::Fallback,
                    format!("resource {token} declaration {ordinal}"),
                    &message,
                ));
                warnings.push(message);
                None
            }
        };
        cached.insert((token.clone(), *ordinal), Declared { name, icon });
    }
    Ok(findings)
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
    ResourceNaming::of(source, context).infos(resources, cancelled)
}

/// What naming recorded resources reads of a source, held apart from it: the
/// content catalogue of its content stack (or why it is unavailable) and the
/// source's resource cache. A preparation that publishes names where it holds
/// no source takes it when it last observed one.
pub struct ResourceNaming {
    catalogue: Result<Arc<Catalogue>, String>,
    cache: Arc<std::sync::Mutex<ContentState>>,
}

impl ResourceNaming {
    pub fn of(source: &Source, context: &LoadContext) -> Self {
        Self {
            catalogue: context
                .content()
                .and_then(|content| catalogue::catalogue(source, content))
                .map_err(|error| error.to_string()),
            cache: source.state::<ContentState>(),
        }
    }

    /// [`resource_infos`] of `resources`.
    pub fn infos(
        &self,
        resources: &[(u32, String)],
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<ResourceInfo>, Vec<String>)> {
        infos(&self.catalogue, &self.cache, resources, cancelled)
    }
}

fn infos(
    catalogue: &Result<Arc<Catalogue>, String>,
    state: &std::sync::Mutex<ContentState>,
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
    let infos = match catalogue {
        Ok(catalogue) => {
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            if !cache
                .content
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(&cached.catalogue, catalogue))
            {
                let mut standing = catalogue.verification.clone();
                standing.extend(catalogue.warnings.iter().cloned());
                cache.content = Some(ContentCache {
                    catalogue: Arc::clone(catalogue),
                    resources: BTreeMap::new(),
                    warnings: standing,
                });
            }
            let content = cache.content.as_mut().expect("content cached");
            resource_content(
                &ContentView::of(catalogue),
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
        targa(colours, false)
    }

    /// A strip across the texture, or down it.
    fn targa(colours: &[[u8; 3]], down: bool) -> Vec<u8> {
        let long = colours.len() as u16 * 2;
        let (width, height) = if down { (2, long) } else { (long, 2) };
        let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        // 32 bits a pixel, top-left origin, 8 alpha bits.
        bytes.extend_from_slice(&[32, 0x28]);
        for y in 0..height {
            for x in 0..width {
                let [red, green, blue] = colours[usize::from(if down { y } else { x } / 2)];
                bytes.extend_from_slice(&[blue, green, red, 255]);
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
        crate::catalogue::texture_tests::write_sprite_program(&base);
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
        let sprites = installed_content_gfx::gfx::read(layers).unwrap();
        let localisation = crate::catalogue::tests::loaded(concat!(
            "l_english:\n",
            " PRODUCTION_MATERIALS_OIL:0 \"Fixture fuel\"\n",
            " PRODUCTION_MATERIALS_UNOBTAINIUM:0 \"Unobtainium\"\n",
        ));
        let guis = installed_content_core::gui::native::read_layers(layers).unwrap();
        let view = ContentView {
            layers,
            sprites: &sprites,
            localisation: &localisation,
            guis: Ok(&guis),
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                Some("Fixture fuel"),
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
        // Two files register the resource row; the registry keeps the last
        // in the content stack's load order.
        let (resources, _) = declared(&modded(&root, false));
        assert_eq!(first_pixel(resources[2].icon.as_deref().unwrap()), BLUE);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_mod_resource_icon_uses_the_padded_fractional_png_frame() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::{from_url, png_bytes, texture_catalogue};
        let image = installed_content_image::Rgba8Image {
            width: 6,
            height: 3,
            pixels: (0..18).flat_map(|x| [x, 20, 30, 200]).collect(),
        };
        let (mut catalogue, _folders) = texture_catalogue(
            &[
                (
                    "interface/mod.gui",
                    "guiTypes = { containerWindowType = { name = resources_info_entry buttonType = { name = resources_icon spriteType = GFX_mod_sheet } } }",
                ),
                (
                    "interface/art.gfx",
                    "spriteTypes = { spriteType = { name = GFX_mod_sheet textureFile = gfx/mod/sheet.png noOfFrames = 3 } }",
                ),
                (
                    "common/resources/mod.txt",
                    "resources = { mod_ore = { icon_frame = 2 } }",
                ),
            ],
            &[("gfx/mod/sheet.png", png_bytes(&image))],
        );
        catalogue.gui_decode = Ok(installed_content_image::native::Decode {
            force_pow2_textures: true,
        });
        let mut cached = Default::default();
        let mut warnings = Vec::new();
        resource_content(
            &ContentView::of(&catalogue),
            &[("mod_ore".into(), 1)],
            &mut cached,
            &mut warnings,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let icon = from_url(cached[&("mod_ore".into(), 1)].icon.as_deref().unwrap());
        assert_eq!((icon.width, icon.height), (2, 4));
        assert_eq!(&icon.pixels[..8], [3, 20, 30, 200, 4, 20, 30, 200]);
        assert_eq!(&icon.pixels[24..], [0; 8]);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn shared_texture_failures_keep_resource_names_and_report_one_cause() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "interface/test.gui",
                "guiTypes = { containerWindowType = { name = resources_info_entry buttonType = { name = resources_icon spriteType = icon } } }",
            ),
            (
                "interface/test.gfx",
                "spriteTypes = { spriteType = { name = icon texturefile = gfx/icon.dds noOfFrames = 2 } }",
            ),
            (
                "common/resources/test.txt",
                "resources = { ore = { icon_frame = 1 } metal = { icon_frame = 2 } }",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n PRODUCTION_MATERIALS_ORE:0 \"Ore\"\n PRODUCTION_MATERIALS_METAL:0 \"Metal\"\n",
            ),
            ("gfx/icon.dds", "invalid DDS"),
        ]);
        for decode in [
            Err("settings.txt is unavailable".into()),
            Ok(Default::default()),
        ] {
            catalogue.gui_decode = decode;
            let mut cached = Default::default();
            let mut warnings = Vec::new();
            let wanted = [("ore".into(), 1), ("metal".into(), 1)];
            let findings = resource_content(
                &ContentView::of(&catalogue),
                &wanted,
                &mut cached,
                &mut warnings,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert_eq!(findings[0].kind, dashboard_core::findings::Kind::Refused);
            assert_eq!(findings[0].subject, "resource icons");
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].starts_with("Resource icons are unavailable:"));
            assert_eq!(cached[&wanted[0]].name.as_deref(), Some("Ore"));
            assert_eq!(cached[&wanted[1]].name.as_deref(), Some("Metal"));
            assert!(cached.values().all(|declared| declared.icon.is_none()));
            let repeated = resource_content(
                &ContentView::of(&catalogue),
                &wanted,
                &mut cached,
                &mut warnings,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(repeated.is_empty());
            assert_eq!(warnings.len(), 1);
        }
    }

    /// The icons, and icon findings, of a catalogue over `files`, `textures` and
    /// the resources `ore` (frame 1), `metal` (2), `gem` (3) and `ingot` (4).
    fn drawn(
        sprite: &str,
        files: &[(&str, String)],
        textures: &[(&str, Vec<u8>)],
    ) -> (Vec<Option<String>>, Vec<dashboard_core::findings::Finding>) {
        let gui = format!(
            "guiTypes = {{ containerWindowType = {{ name = resources_info_entry buttonType = {{ name = resources_icon spriteType = {sprite} }} }} }}"
        );
        let mut all: Vec<(&str, &str)> = vec![
            ("interface/trade.gui", &gui),
            (
                "common/resources/fixture.txt",
                "resources = { ore = { icon_frame = 1 } metal = { icon_frame = 2 } gem = { icon_frame = 3 } ingot = { icon_frame = 4 } }",
            ),
        ];
        all.extend(files.iter().map(|(path, text)| (*path, text.as_str())));
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&all);
        for (path, bytes) in textures {
            write(&folder.0, path, bytes);
        }
        crate::catalogue::texture_tests::write_replacement_textures(&folder.0);
        let wanted: Vec<(String, u32)> = ["ore", "metal", "gem", "ingot"]
            .iter()
            .map(|token| (token.to_string(), 1))
            .collect();
        let mut cached = BTreeMap::new();
        let findings = resource_content(
            &ContentView::of(&catalogue),
            &wanted,
            &mut cached,
            &mut Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let icons = wanted.iter().map(|key| cached[key].icon.clone()).collect();
        // The fixture's resources have no names; only icon findings are kept.
        let findings = findings
            .into_iter()
            .filter(|finding| finding.kind != dashboard_core::findings::Kind::Content)
            .collect();
        (icons, findings)
    }

    #[test]
    fn resource_icons_draw_the_native_sprite_across_down_animated_and_past_the_strip() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        for (kind, down) in [
            ("spriteType", false),
            ("spriteType", true),
            ("frameAnimatedSpriteType", false),
            ("frameAnimatedSpriteType", true),
        ] {
            let direction = if down { "yes" } else { "no" };
            let gfx = format!(
                "spriteTypes = {{ {kind} = {{ name = GFX_fixture_strip texturefile = gfx/strip.tga noOfFrames = 3 frameDirectionY = {direction} }} }}"
            );
            let (icons, findings) = drawn(
                "GFX_fixture_strip",
                &[("interface/strip.gfx", gfx)],
                &[("gfx/strip.tga", targa(&[RED, GREEN, BLUE], down))],
            );
            let pixels: Vec<_> = icons
                .iter()
                .map(|icon| first_pixel(icon.as_deref().unwrap()))
                .collect();
            // A frame past the strip keeps its offset; the program's clamped
            // sampler reads the strip's last texels, as the game draws it.
            assert_eq!(pixels, [RED, GREEN, BLUE, BLUE], "{kind} {direction}");
            assert!(findings.is_empty(), "{findings:?}");
        }
        // A kind the renderer cannot draw is refused once, by name, for every
        // resource; no strip is cropped in its place.
        let gfx = "spriteTypes = { corneredTileSpriteType = { name = GFX_fixture_tile texturefile = gfx/strip.tga } }";
        let (icons, findings) = drawn(
            "GFX_fixture_tile",
            &[("interface/tile.gfx", gfx.into())],
            &[("gfx/strip.tga", strip(&[RED]))],
        );
        assert!(icons.iter().all(Option::is_none));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject, "resource icons");
        assert!(
            findings[0].message.contains("GFX_fixture_tile")
                && findings[0]
                    .message
                    .contains("needs native sprite rendering"),
            "{findings:?}"
        );
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn resource_icons_execute_a_mod_s_effects_masks_and_animations() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::{
            from_url, replacement_programs, replacement_sprites,
        };
        for (declaration, pixel) in replacement_sprites("GFX_fixture_icon") {
            let mut files = replacement_programs();
            files.push((
                "interface/icon.gfx",
                format!("spriteTypes = {{ {declaration} }}"),
            ));
            let (icons, findings) = drawn("GFX_fixture_icon", &files, &[]);
            assert!(findings.is_empty(), "{declaration}: {findings:?}");
            let icon = from_url(icons[0].as_deref().unwrap());
            assert!(
                icon.pixels.chunks_exact(4).all(|rgba| rgba == pixel),
                "{declaration}: {:?}",
                icon.pixels
            );
        }
    }
}

#[cfg(test)]
mod naming_tests {
    use super::resource_name;

    /// The name is `PRODUCTION_MATERIALS_` and the upper-cased token through
    /// the plain localiser, loaded as content loads.
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
                resource_name(&localisation, token, &mut warnings, &mut Vec::new()).as_deref(),
                Some("Oil [ROOT.GetName]")
            );
        }
        assert_eq!(
            resource_name(&localisation, "steel", &mut warnings, &mut Vec::new()).as_deref(),
            Some("")
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            resource_name(&localisation, "tungsten", &mut warnings, &mut Vec::new()).as_deref(),
            Some("PRODUCTION_MATERIALS_TUNGSTEN")
        );
        assert_eq!(
            resource_name(&localisation, "coal", &mut warnings, &mut Vec::new()),
            None
        );
        // The "C" locale's toupper leaves a byte above 0x7F.
        assert_eq!(
            resource_name(&localisation, "stål", &mut warnings, &mut Vec::new()).as_deref(),
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
