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
    /// An artwork URL; null when content has no icon for it.
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
/// from; one per source and kind of catalogue ([`catalogue::ByCatalogue`]: stored payloads
/// read the recording's own), whichever resource product asks.
#[derive(Default)]
struct ContentState {
    content: Option<ContentCache>,
}

struct ContentCache {
    catalogue: Arc<Catalogue>,
    /// By token and which declaration of it (from 1).
    resources: BTreeMap<(String, u32), catalogue::Drawn<Declared>>,
    /// The standing notes and every warning reading names and icons made, in order, each with
    /// what it is about.
    warnings: Vec<(Note, String)>,
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
    cached: &mut BTreeMap<(String, u32), catalogue::Drawn<Declared>>,
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    resource_content_noted(
        view,
        wanted,
        cached,
        &mut |_, warning| warnings.push(warning),
        cancelled,
    )
}

/// What a resource warning is about: the resource's name, or the art its icon is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Note {
    Name,
    Art,
}

/// [`resource_content`], each warning given to `note` with what it is about, in order.
fn resource_content_noted(
    view: &ContentView,
    wanted: &[(String, u32)],
    cached: &mut BTreeMap<(String, u32), catalogue::Drawn<Declared>>,
    note: &mut dyn FnMut(Note, String),
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
            for warning in notes {
                note(Note::Art, warning);
            }
            Some(icons)
        }
        Err(error) => {
            let message = format!("Resource icons are unavailable: {error}");
            findings.push(Finding::new(Kind::Refused, "resource icons", &message));
            note(Note::Art, message);
            None
        }
    };
    let decode = match (&icons, view.gui_decode) {
        (Some(_), Ok(decode)) => Some(*decode),
        (Some(_), Err(why)) => {
            let message = format!("Resource icons are unavailable: {why}");
            findings.push(Finding::new(Kind::Refused, "resource icons", &message));
            note(Note::Art, message);
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
            for warning in notes {
                note(Note::Name, warning);
            }
        }
        let Some(icons) = &icons else {
            let declared = catalogue::Drawn::new(|| Declared { name, icon: None });
            cached.insert((token.clone(), *ordinal), declared);
            continue;
        };
        let declared = catalogue::Drawn::new(|| {
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
                        note(Note::Art, message);
                        None
                    }
                },
                (Some(_), None) => None,
                (None, _) => {
                    let message =
                        format!("{token}: the recorded content gives no usable icon frame.");
                    findings.push(Finding::new(
                        Kind::Fallback,
                        format!("resource {token} declaration {ordinal}"),
                        &message,
                    ));
                    note(Note::Art, message);
                    None
                }
            };
            Declared { name, icon }
        });
        // What the draw fell back on reaches this request as it does every later one.
        declared.get();
        cached.insert((token.clone(), *ordinal), declared);
    }
    Ok(findings)
}

/// Every recorded resource (native index and token, in the game's order) with its
/// content name and icon, and the warnings that go with them. Without the game
/// folders each keeps its token and has no icon, and a warning says why. Names are the
/// recording's; while the request shows visual mods, icons are drawn as it shows them, each one
/// operation ([`present_stored`]), so where the selection cannot draw one the recording's own
/// is drawn, named, as every resource widget draws it, and the art warnings are the chosen
/// draws'. Art that cannot be drawn at all leaves the names, by why.
pub fn resource_infos(
    source: &Source,
    context: &LoadContext,
    resources: &[(u32, String)],
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ResourceInfo>, Vec<String>)> {
    let naming = ResourceNaming::stored(source, context);
    let Some(content) = context
        .content()
        .ok()
        .filter(|content| content.presentation.shows_mods())
    else {
        return naming.infos(resources, cancelled);
    };
    let (mut infos, mut warnings) =
        infos(&naming.catalogue, &naming.cache, resources, cancelled, false)?;
    match present_icons(source, Some(content), &mut infos, cancelled) {
        Ok(refused) => warnings.extend(
            refused
                .into_iter()
                .map(|why| format!("Resource icons are unavailable: {why}")),
        ),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            for info in &mut infos {
                info.icon = None;
            }
            warnings.push(format!("Resource icons are unavailable: {error}"));
        }
    }
    if infos.iter().any(|resource| resource.icon.is_some()) {
        warnings.push(ARTWORK_NOTE.to_owned());
    }
    Ok((infos, warnings))
}

/// What a reply showing resource icons says of where they come from.
const ARTWORK_NOTE: &str = "Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.";

/// [`resource_infos`] for payloads the prepared store keeps ([`ResourceNaming::stored`]).
pub fn stored_resource_infos(
    source: &Source,
    context: &LoadContext,
    resources: &[(u32, String)],
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ResourceInfo>, Vec<String>)> {
    ResourceNaming::stored(source, context).infos(resources, cancelled)
}

/// Draws the icons of resources a prepared payload stored (drawn with the recording's art, as
/// preparation is) as the request admitted under `content` shows them, when it shows visual
/// mods; without them the stored icons are already the ones it shows.
pub fn present_stored(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    resources: &mut [ResourceInfo],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    present_icons(source, content, resources, cancelled).map(|_| ())
}

/// [`present_stored`], with why each icon neither the selection nor the recording draws is
/// unavailable, each reason once.
fn present_icons(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    resources: &mut [ResourceInfo],
    cancelled: &AtomicBool,
) -> io::Result<BTreeSet<String>> {
    let mut refused = BTreeSet::new();
    let Some((admitted, recorded)) = catalogue::presenting(source, content)? else {
        return Ok(refused);
    };
    let state = source.state::<catalogue::ByCatalogue<ResourceArt>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let ResourceArt { frames, drawn } = state.of(&admitted);
    if !frames
        .as_ref()
        .is_some_and(|(held, _)| Arc::ptr_eq(held, &recorded))
    {
        *frames = Some((Arc::clone(&recorded), icon_frames(&recorded.content.layers)?));
    }
    let (_, frames) = frames.as_ref().expect("resource frames read");
    let drawn = catalogue::Redrawn::bind(drawn, &admitted, &recorded);
    for resource in resources {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Resource request cancelled",
            ));
        }
        let key = (resource.token.clone(), resource.ordinal);
        let shown = drawn.get(
            key,
            || format!("resource {} icon", resource.token),
            |catalogue| resource_icon(catalogue, frames, &resource.token, resource.ordinal),
        );
        resource.icon = match shown {
            Ok(icon) => Some(icon.clone()),
            Err(why) => {
                refused.insert(why.clone());
                None
            }
        };
    }
    Ok(refused)
}

/// The recorded frames of each resource declaration and the icons drawn for one selection; one
/// per kind of catalogue ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct ResourceArt {
    frames: Option<(Arc<Catalogue>, BTreeMap<String, Vec<Option<u32>>>)>,
    drawn: Option<catalogue::Redrawn<(String, u32), String>>,
}

/// Each resource declaration's one-based frame in the trade view's icon, per token in load
/// order: recorded facts.
fn icon_frames(layers: &ResolvedLayers) -> io::Result<BTreeMap<String, Vec<Option<u32>>>> {
    let mut frames: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
    for resource in installed_content_core::resources::read(layers)?.0 {
        frames
            .entry(resource.token)
            .or_default()
            .push(resource.icon_frame);
    }
    Ok(frames)
}

/// A resource declaration's icon drawn through `catalogue`: the trade view's icon sprite, as its
/// GUI selects it, at the declaration's recorded frame.
fn resource_icon(
    catalogue: &Catalogue,
    frames: &BTreeMap<String, Vec<Option<u32>>>,
    token: &str,
    ordinal: u32,
) -> Result<String, String> {
    let guis = catalogue.guis().map_err(|error| error.to_string())?;
    let name = installed_content_core::gui::native::window_button_sprite(
        guis,
        RESOURCE_ROW,
        RESOURCE_ICON,
        &mut Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let sprite =
        catalogue::created_gui_sprite(&catalogue.sprites, &name).map_err(|e| e.to_string())?;
    let frame = ordinal
        .checked_sub(1)
        .and_then(|index| frames.get(token)?.get(index as usize).copied().flatten())
        .and_then(|frame| frame.checked_sub(1))
        .ok_or_else(|| format!("{token}: the recorded content gives no usable icon frame."))?;
    let decode = *catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, frame, ICON_EDGE)
        .map_err(|error| error.to_string())
}

/// [`present_stored`] over a serialized reply's `resources` list, left as it is when the request
/// shows no visual mods or the reply has none.
pub fn present_stored_json(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    resources: &mut serde_json::Value,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    if resources.is_null() || !content.is_some_and(|content| content.presentation.shows_mods()) {
        return Ok(());
    }
    let mut infos: Vec<ResourceInfo> = serde_json::from_value(resources.take())?;
    present_stored(source, content, &mut infos, cancelled)?;
    *resources = serde_json::to_value(infos)?;
    Ok(())
}

/// What naming recorded resources reads of a source, held apart from it: the
/// content catalogue of its content stack (or why it is unavailable) and the
/// source's resource cache. A preparation that publishes names where it holds
/// no source takes it when it last observed one.
pub struct ResourceNaming {
    catalogue: Result<Arc<Catalogue>, String>,
    cache: Arc<std::sync::Mutex<catalogue::ByCatalogue<ContentState>>>,
}

impl ResourceNaming {
    /// Naming as the recording gives it: names, warnings and the recording's own icons
    /// (`catalogue::recorded_catalogue`), which a reply showing visual mods draws again as it
    /// shows them ([`present_stored`]); what preparation publishes for the prepared store.
    pub fn stored(source: &Source, context: &LoadContext) -> Self {
        Self {
            catalogue: context
                .content()
                .and_then(|content| catalogue::recorded_catalogue(source, content))
                .map_err(|error| error.to_string()),
            cache: source.state(),
        }
    }

    /// [`resource_infos`] of `resources`, with the recording's icons and their warnings.
    pub fn infos(
        &self,
        resources: &[(u32, String)],
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<ResourceInfo>, Vec<String>)> {
        infos(&self.catalogue, &self.cache, resources, cancelled, true)
    }
}

/// The names and the recording's icons of `resources`, with the warnings reading them made: the
/// art warnings and the artwork note only with `art`, for a reply that shows these icons.
fn infos(
    catalogue: &Result<Arc<Catalogue>, String>,
    state: &std::sync::Mutex<catalogue::ByCatalogue<ContentState>>,
    resources: &[(u32, String)],
    cancelled: &AtomicBool,
    art: bool,
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
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            let cache = state.of(catalogue);
            if !cache
                .content
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(&cached.catalogue, catalogue))
            {
                let standing = catalogue
                    .verification
                    .iter()
                    .chain(&catalogue.warnings)
                    .map(|warning| (Note::Name, warning.clone()))
                    .collect();
                cache.content = Some(ContentCache {
                    catalogue: Arc::clone(catalogue),
                    resources: BTreeMap::new(),
                    warnings: standing,
                });
            }
            let content = cache.content.as_mut().expect("content cached");
            let kept = &mut content.warnings;
            resource_content_noted(
                &ContentView::of(catalogue),
                &declarations,
                &mut content.resources,
                &mut |about, warning| kept.push((about, warning)),
                cancelled,
            )?;
            warnings.extend(
                content
                    .warnings
                    .iter()
                    .filter(|(about, _)| art || *about == Note::Name)
                    .map(|(_, warning)| warning.clone()),
            );
            let infos: Vec<ResourceInfo> = declarations
                .iter()
                .enumerate()
                .map(|(index, key)| {
                    info(index, content.resources.get(key).map(catalogue::Drawn::get))
                })
                .collect();
            if art && infos.iter().any(|resource| resource.icon.is_some()) {
                warnings.push(ARTWORK_NOTE.to_owned());
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

    /// The first pixel of the image an artwork URL names.
    fn first_pixel(url: &str) -> [u8; 3] {
        let bytes = dashboard_text::artwork::url_bytes(url).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(&*bytes))
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
            wanted.iter().map(|key| cached[key].get().clone()).collect(),
            warnings,
        )
    }

    /// Icons a prepared payload stored with the recording's art are drawn at query time as the
    /// request shows them: a selected mod's artwork, the recording's (named) where the mod's
    /// cannot be drawn, on every reply, and the stored ones untouched without a selection.
    #[test]
    fn stored_resource_icons_follow_the_selection_at_query_time() {
        use crate::test_fixtures::visual_mods::{
            BLUE, Machine, RED, fallback_line, first_pixel, png, select,
        };
        let machine = Machine::new("stored_resources");
        machine.write(
            "interface/resources.gui",
            b"guiTypes = { containerWindowType = { name = resources_info_entry               buttonType = { name = resources_icon spriteType = GFX_strip } } }",
        );
        machine.write(
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_strip textureFile = gfx/strip.png } }",
        );
        machine.write("gfx/strip.png", &png(RED));
        machine.write("common/resources/00.txt", b"resources = { oil = { icon_frame = 1 } }");
        machine.write_mod("art", "gfx/strip.png", &png(BLUE));
        // The mod replaces the recording's declaration with one whose texture does not load.
        machine.write_mod(
            "broken",
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_strip textureFile = gfx/broken.png } }",
        );
        machine.write_mod("broken", "gfx/broken.png", b"not a png");
        let (source, mut options) = machine.open();
        select(&source, &mut options, &[]);
        let cancelled = AtomicBool::new(false);
        let preparation = LoadContext {
            content: source.admit(crate::source::Work::Recorded),
        };
        let (stored, _) =
            stored_resource_infos(&source, &preparation, &[(7, "oil".into())], &cancelled).unwrap();
        assert_eq!(first_pixel(stored[0].icon.as_deref().unwrap()), RED);
        let shown = |options: &crate::products::ContentOptions| {
            let mut resources = stored.clone();
            let ((), line) = fallback_line(|| {
                present_stored(&source, Some(options), &mut resources, &cancelled).unwrap()
            });
            (first_pixel(resources[0].icon.as_deref().unwrap()), line)
        };
        assert_eq!(shown(&options), (RED, None), "without a selection, as stored");
        select(&source, &mut options, &["art"]);
        assert_eq!(shown(&options), (BLUE, None));
        select(&source, &mut options, &["broken"]);
        for _ in 0..2 {
            let (colour, line) = shown(&options);
            assert_eq!(colour, RED, "the recording's own");
            assert!(line.as_ref().is_some_and(|line| line.contains("GFX_strip")), "{line:?}");
        }
        // The stored entry keeps its own number and ordinal.
        let mut resources = stored.clone();
        present_stored(&source, Some(&options), &mut resources, &cancelled).unwrap();
        assert_eq!((resources[0].id, resources[0].ordinal), (7, 1));
    }

    /// The recording's trade view selects sprite A and a mod's selects B, which the mod cannot
    /// draw though the recording's B could: the recording's own choice, A, is drawn, never the
    /// mod's choice patched with the recording's B.
    #[test]
    fn a_failed_selection_makes_the_recordings_own_choice_again() {
        use crate::test_fixtures::visual_mods::{
            GREEN, Machine, RED, fallback_line, first_pixel, png, select,
        };
        let gui = |sprite: &str| {
            format!(
                "guiTypes = {{ containerWindowType = {{ name = resources_info_entry \
                 buttonType = {{ name = resources_icon spriteType = {sprite} }} }} }}"
            )
        };
        let machine = Machine::new("resource_selection");
        machine.write("interface/resources.gui", gui("GFX_a").as_bytes());
        machine.write(
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png } \
              spriteType = { name = GFX_b textureFile = gfx/b.png } }",
        );
        machine.write("gfx/a.png", &png(RED));
        machine.write("gfx/b.png", &png(GREEN));
        machine.write("common/resources/00.txt", b"resources = { oil = { icon_frame = 1 } }");
        machine.write_mod("selector", "interface/resources.gui", gui("GFX_b").as_bytes());
        // The mod's declarations replace the recording's file: its B does not load.
        machine.write_mod(
            "selector",
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png }               spriteType = { name = GFX_b textureFile = gfx/broken.png } }",
        );
        machine.write_mod("selector", "gfx/broken.png", b"not a png");
        let (source, mut options) = machine.open();
        select(&source, &mut options, &["selector"]);
        let cancelled = AtomicBool::new(false);
        for _ in 0..2 {
            let mut resources = vec![ResourceInfo {
                id: 1,
                token: "oil".into(),
                ordinal: 1,
                name: None,
                icon: Some("stored".into()),
            }];
            let ((), line) = fallback_line(|| {
                present_stored(&source, Some(&options), &mut resources, &cancelled).unwrap()
            });
            assert_eq!(first_pixel(resources[0].icon.as_deref().unwrap()), RED);
            assert!(line.is_some_and(|line| line.contains("resource oil icon")));
        }
    }

    /// A reply that names resources when it answers (faction details) draws their icons as the
    /// prepared families do: the mod's selector picks B, which the mod cannot draw, so the
    /// recording's own choice, A, is drawn and named on every reply; the names stay the
    /// recording's. Without a selection the reply is drawn as it is.
    #[test]
    fn resources_named_when_a_reply_answers_make_the_recordings_own_choice_again() {
        use crate::test_fixtures::visual_mods::{
            GREEN, Machine, RED, fallback_line, first_pixel, png, select,
        };
        let gui = |sprite: &str| {
            format!(
                "guiTypes = {{ containerWindowType = {{ name = resources_info_entry                  buttonType = {{ name = resources_icon spriteType = {sprite} }} }} }}"
            )
        };
        let machine = Machine::new("resource_reply_selection");
        machine.write("interface/resources.gui", gui("GFX_a").as_bytes());
        machine.write(
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png }               spriteType = { name = GFX_b textureFile = gfx/b.png } }",
        );
        machine.write("gfx/a.png", &png(RED));
        machine.write("gfx/b.png", &png(GREEN));
        machine.write("common/resources/00.txt", b"resources = { oil = { icon_frame = 1 } }");
        machine.write_mod("selector", "interface/resources.gui", gui("GFX_b").as_bytes());
        machine.write_mod(
            "selector",
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png }               spriteType = { name = GFX_b textureFile = gfx/broken.png } }",
        );
        machine.write_mod("selector", "gfx/broken.png", b"not a png");
        let (source, mut options) = machine.open();
        let cancelled = AtomicBool::new(false);
        let reply = |options: &crate::products::ContentOptions| {
            let context = LoadContext {
                content: Some(options.clone()),
            };
            let ((infos, _), line) = fallback_line(|| {
                resource_infos(&source, &context, &[(3, "oil".into())], &cancelled).unwrap()
            });
            assert_eq!((infos[0].id, infos[0].ordinal), (3, 1));
            (first_pixel(infos[0].icon.as_deref().unwrap()), line)
        };
        select(&source, &mut options, &[]);
        assert_eq!(reply(&options), (RED, None));
        select(&source, &mut options, &["selector"]);
        for _ in 0..2 {
            let (colour, line) = reply(&options);
            assert_eq!(colour, RED, "the recording's own choice");
            assert!(line.is_some_and(|line| line.contains("resource oil icon")));
        }
    }

    /// While mods are shown, a reply naming resources keeps only its chosen draws' art warnings:
    /// a recording without the trade view's icon binding that a mod supplies draws the mod's
    /// icon with no word of the recording's missing one; and art that cannot be drawn at all (the
    /// recorded definitions unreadable) leaves the names, by why, rather than failing the reply.
    #[test]
    fn resources_named_when_a_reply_answers_keep_their_names_and_the_chosen_art_warnings() {
        use crate::test_fixtures::visual_mods::{BLUE, Machine, first_pixel, png, select};
        let machine = Machine::new("resource_reply_warnings");
        machine.write("common/resources/00.txt", b"resources = { oil = { icon_frame = 1 } }");
        machine.write_mod(
            "binding",
            "interface/resources.gui",
            b"guiTypes = { containerWindowType = { name = resources_info_entry               buttonType = { name = resources_icon spriteType = GFX_strip } } }",
        );
        machine.write_mod(
            "binding",
            "interface/resources.gfx",
            b"spriteTypes = { spriteType = { name = GFX_strip textureFile = gfx/strip.png } }",
        );
        machine.write_mod("binding", "gfx/strip.png", &png(BLUE));
        let (source, mut options) = machine.open();
        let cancelled = AtomicBool::new(false);
        let reply = |options: &crate::products::ContentOptions| {
            resource_infos(
                &source,
                &LoadContext {
                    content: Some(options.clone()),
                },
                &[(3, "oil".into())],
                &cancelled,
            )
        };
        select(&source, &mut options, &[]);
        let (infos, warnings) = reply(&options).unwrap();
        assert_eq!(infos[0].icon, None);
        assert!(
            warnings.iter().any(|why| why.starts_with("Resource icons are unavailable")),
            "{warnings:?}"
        );
        select(&source, &mut options, &["binding"]);
        let (infos, warnings) = reply(&options).unwrap();
        assert_eq!(first_pixel(infos[0].icon.as_deref().unwrap()), BLUE);
        assert!(
            !warnings.iter().any(|why| why.starts_with("Resource icons are unavailable")),
            "{warnings:?}"
        );
        // Recorded definitions listed when the content was mounted and gone when the art reads
        // them, on a source that has drawn nothing yet.
        let (source, mut options) = machine.open();
        select(&source, &mut options, &["binding"]);
        catalogue::catalogue(&source, &options).unwrap();
        std::fs::remove_file(machine.root.join("common/resources/00.txt")).unwrap();
        let (infos, warnings) = resource_infos(
            &source,
            &LoadContext {
                content: Some(options.clone()),
            },
            &[(3, "oil".into())],
            &cancelled,
        )
        .unwrap();
        assert_eq!(infos[0].token, "oil");
        assert!(infos[0].name.is_some(), "{infos:?}");
        assert_eq!(infos[0].icon, None);
        assert!(
            warnings.iter().any(|why| why.starts_with("Resource icons are unavailable")),
            "{warnings:?}"
        );
    }

    /// The A/B selection, with an ordinary draw of the mod's B (which falls back to the
    /// recording's B) in either order around the strict redraw: neither reads the other's
    /// image, so the redraw makes the recording's own choice, A, and the ordinary draw still
    /// falls back to B.
    #[test]
    fn strict_and_ordinary_draws_keep_their_images_apart_in_either_order() {
        use crate::test_fixtures::visual_mods::{
            GREEN, Machine, RED, first_pixel, png, select,
        };
        let gui = |sprite: &str| {
            format!(
                "guiTypes = {{ containerWindowType = {{ name = resources_info_entry \
                 buttonType = {{ name = resources_icon spriteType = {sprite} }} }} }}"
            )
        };
        for ordinary_first in [true, false] {
            let machine = Machine::new(if ordinary_first {
                "strict_images_ordinary_first"
            } else {
                "strict_images_strict_first"
            });
            machine.write("interface/resources.gui", gui("GFX_a").as_bytes());
            machine.write(
                "interface/resources.gfx",
                b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png } \
                  spriteType = { name = GFX_b textureFile = gfx/b.png } }",
            );
            machine.write("gfx/a.png", &png(RED));
            machine.write("gfx/b.png", &png(GREEN));
            machine.write("common/resources/00.txt", b"resources = { oil = { icon_frame = 1 } }");
            machine.write_mod("selector", "interface/resources.gui", gui("GFX_b").as_bytes());
            machine.write_mod(
                "selector",
                "interface/resources.gfx",
                b"spriteTypes = { spriteType = { name = GFX_a textureFile = gfx/a.png } \
                  spriteType = { name = GFX_b textureFile = gfx/broken.png } }",
            );
            machine.write_mod("selector", "gfx/broken.png", b"not a png");
            let (source, mut options) = machine.open();
            select(&source, &mut options, &["selector"]);
            let admitted = catalogue::catalogue(&source, &options).unwrap();
            // What an ordinary query-time widget draws of the mod's B.
            let ordinary = || {
                let sprite = admitted.sprites.find("GFX_b").unwrap().unwrap();
                catalogue::sprite_frame_png(
                    &admitted.content.layers,
                    sprite,
                    *admitted.gui_decode.as_ref().unwrap(),
                    0,
                    ICON_EDGE,
                )
                .unwrap()
            };
            let strict = || {
                let mut resources = vec![ResourceInfo {
                    id: 1,
                    token: "oil".into(),
                    ordinal: 1,
                    name: None,
                    icon: None,
                }];
                present_stored(&source, Some(&options), &mut resources, &AtomicBool::new(false))
                    .unwrap();
                first_pixel(resources[0].icon.as_deref().unwrap())
            };
            if ordinary_first {
                assert_eq!(first_pixel(&ordinary()), GREEN, "the recording's B");
                assert_eq!(strict(), RED, "the recording's own choice");
            } else {
                assert_eq!(strict(), RED, "the recording's own choice");
                assert_eq!(first_pixel(&ordinary()), GREEN, "the recording's B");
            }
        }
    }

    /// A selected mod's interface file that cannot be read (gone after the mods were mounted):
    /// an ordinary read of the GUIs falls back to the recording's, and a strict read fails on
    /// that file; in either order, neither leaves its result for the other.
    #[test]
    fn strict_and_ordinary_gui_registries_are_kept_apart_in_either_order() {
        use crate::test_fixtures::visual_mods::{Machine, png, select, RED};
        for ordinary_first in [true, false] {
            let machine = Machine::new(if ordinary_first {
                "strict_guis_ordinary_first"
            } else {
                "strict_guis_strict_first"
            });
            machine.write(
                "interface/resources.gui",
                b"guiTypes = { containerWindowType = { name = resources_info_entry } }",
            );
            machine.write_mod("unreadable", "gfx/unrelated.png", &png(RED));
            machine.write_mod(
                "unreadable",
                "interface/broken.gui",
                b"guiTypes = { containerWindowType = { name = broken } }",
            );
            let (source, mut options) = machine.open();
            select(&source, &mut options, &["unreadable"]);
            let admitted = catalogue::catalogue(&source, &options).unwrap();
            // Listed when the mods were mounted, unreadable now.
            std::fs::remove_file(machine.mods.join("unreadable/interface/broken.gui")).unwrap();
            let ordinary = || admitted.guis().is_ok();
            // The strict read fails on that file itself, not on anything else.
            let strict = || {
                let error = installed_content_core::content::strict(|| {
                    admitted.guis().map(|_| ()).map_err(|error| error.to_string())
                })
                .expect_err("the mods' GUIs do not read");
                assert!(error.contains("broken.gui"), "{error}");
            };
            if ordinary_first {
                assert!(ordinary(), "the recording's GUIs");
                strict();
            } else {
                strict();
                assert!(ordinary(), "the recording's GUIs");
            }
        }
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
        let icon = from_url(
            cached[&("mod_ore".into(), 1)]
                .get()
                .icon
                .as_deref()
                .unwrap(),
        );
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
            assert_eq!(cached[&wanted[0]].get().name.as_deref(), Some("Ore"));
            assert_eq!(cached[&wanted[1]].get().name.as_deref(), Some("Metal"));
            assert!(
                cached
                    .values()
                    .all(|declared| declared.get().icon.is_none())
            );
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
        let icons = wanted
            .iter()
            .map(|key| cached[key].get().icon.clone())
            .collect();
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
