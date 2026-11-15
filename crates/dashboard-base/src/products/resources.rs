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
/// one-based frame in it.
struct Icons {
    sprite: installed_content_gfx::gfx::SpriteType,
    frames: Frames,
}

/// Each numbered declaration's one-based icon frame, per token in load order, and the
/// first file that stops short, after which no declaration has a known ordinal.
#[derive(Default)]
struct Frames {
    by_token: BTreeMap<String, Vec<Option<u32>>>,
    gap: Option<String>,
}

impl Frames {
    fn new(resources: &installed_content_core::resources::Resources) -> Self {
        let mut by_token: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
        for resource in resources.numbered() {
            by_token
                .entry(resource.token.clone())
                .or_default()
                .push(resource.icon_frame);
        }
        Self {
            by_token,
            gap: resources.gap().map(ToString::to_string),
        }
    }

    /// The `ordinal`th (one-based) declaration's frame of `token`: `Ok(None)` when it has
    /// none or the content has no such declaration, the gap when the content leaves it unknown.
    fn of(&self, token: &str, ordinal: u32) -> Result<Option<u32>, String> {
        let declared = ordinal
            .checked_sub(1)
            .and_then(|index| self.by_token.get(token)?.get(index as usize));
        match (declared, &self.gap) {
            (Some(frame), _) => Ok(*frame),
            (None, Some(gap)) if ordinal > 0 => Err(format!(
                "{token}: declaration {ordinal} has no known place in the content: {gap}"
            )),
            (None, _) => Ok(None),
        }
    }
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
    Ok((
        Icons {
            sprite,
            frames: Frames::new(&resources),
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
    // Draw every wanted frame of the shared sprite in one batch. A refused
    // batch leaves each frame to its own draw below, which names the refusal.
    if let (Some(icons), Some(decode)) = (&icons, decode) {
        let frames: BTreeSet<u32> = missing
            .iter()
            .filter_map(|(token, ordinal)| icons.frames.of(token, *ordinal).ok().flatten())
            .map(|frame| frame - 1)
            .collect();
        let requests: Vec<_> = frames.iter().map(|frame| (&icons.sprite, *frame)).collect();
        let _ = catalogue::sprite_frames_png(view.layers, &requests, decode, ICON_EDGE);
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
                icons.frames.of(token, *ordinal),
                decode.filter(|_| !refused),
            ) {
                (Ok(Some(frame)), Some(decode)) => match catalogue::sprite_frame_png(
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
                (Ok(Some(_)), None) => None,
                (Err(message), _) => {
                    findings.push(Finding::new(
                        Kind::Refused,
                        format!("resource {token} declaration {ordinal}"),
                        &message,
                    ));
                    note(Note::Art, message);
                    None
                }
                (Ok(None), _) => {
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
    frames: Option<(Arc<Catalogue>, Frames)>,
    drawn: Option<catalogue::Redrawn<(String, u32), String>>,
}

/// Each resource declaration's one-based frame in the trade view's icon: recorded facts.
fn icon_frames(layers: &ResolvedLayers) -> io::Result<Frames> {
    Ok(Frames::new(&installed_content_core::resources::read(layers)?.0))
}

/// A resource declaration's icon drawn through `catalogue`: the trade view's icon sprite, as its
/// GUI selects it, at the declaration's recorded frame.
fn resource_icon(
    catalogue: &Catalogue,
    frames: &Frames,
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
    let frame = frames
        .of(token, ordinal)?
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
