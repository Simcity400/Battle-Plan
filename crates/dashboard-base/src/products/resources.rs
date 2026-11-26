
use super::LoadContext;
use crate::{
    Source,
    catalogue::{self, Catalogue},
};
use content_reader_localisation::localisation::Localisation;
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

pub const RESOURCE_DECIMALS: u8 = 5;
const ICON_EDGE: u32 = 48;
const RESOURCE_ROW: &str = "resources_info_entry";
const RESOURCE_ICON: &str = "resources_icon";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    #[doc = " The native index (`ResourceDefinitions.id`)."]
    pub id: u32,
    pub token: String,
    #[doc = " Which declaration of its token this is, from 1, in the game's order."]
    pub ordinal: u32,
    pub name: Option<String>,
    #[doc = " An artwork URL; null when content has no icon for it."]
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Learned {
    ids: BTreeMap<u32, String>,
}

impl Learned {
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

    pub fn list(&self) -> Vec<(u32, String)> {
        self.ids
            .iter()
            .map(|(id, token)| (*id, token.clone()))
            .collect()
    }
}

fn ordinals(resources: &[(u32, String)]) -> Vec<u32> {
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

#[derive(Default)]
struct ContentState {
    content: Option<ContentCache>,
}

struct ContentCache {
    catalogue: Arc<Catalogue>,
    resources: BTreeMap<(String, u32), catalogue::Drawn<Declared>>,
    warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Declared {
    name: Option<String>,
    icon: Option<String>,
}

struct Icons {
    sprite: content_reader_gfx::gfx::SpriteType,
    frames: Frames,
}

#[derive(Default)]
struct Frames {
    by_token: BTreeMap<String, Vec<Option<u32>>>,
    gap: Option<String>,
}

impl Frames {
    fn new(resources: &content_reader_key_names::resources::Resources) -> Self {
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

fn icons(catalogue: &Catalogue) -> io::Result<(Icons, Vec<String>)> {
    let mut warnings = Vec::new();
    let guis = catalogue
        .guis()
        .map_err(|error| crate::invalid(error.to_string()))?;
    let name = content_reader_gui::gui::native::window_button_sprite(
        guis,
        RESOURCE_ROW,
        RESOURCE_ICON,
        &mut warnings,
    )?;
    let sprite = catalogue::created_gui_sprite(&catalogue.sprites, &name)?.clone();
    let (resources, notes) = content_reader_key_names::resources::read(&catalogue.content.layers)?;
    warnings.extend(notes.into_iter().map(|(_, message)| message));
    Ok((
        Icons {
            sprite,
            frames: Frames::new(&resources),
        },
        warnings,
    ))
}

fn resource_name(
    localisation: &Localisation,
    token: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let upper = content_reader_localisation::localisation::crt_case_text(token, true, false);
    let key = format!("PRODUCTION_MATERIALS_{upper}");
    if localisation.get(&key).is_none() {
        warnings.push(format!("{token}: localisation {key} is missing"));
        return None;
    }
    let (name, notes) = catalogue::localised_details(
        localisation,
        &key,
        &[],
        catalogue::View::Scripted,
        dashboard_core::findings::Kind::Unsupported,
    );
    warnings.extend(
        notes
            .iter()
            .filter(|finding| finding.kind != dashboard_core::findings::Kind::Content)
            .map(|finding| format!("{token}: no name: {}", finding.message)),
    );
    name
}

fn load_resource_content(
    catalogue: &Catalogue,
    wanted: &[(String, u32)],
    cached: &mut BTreeMap<(String, u32), catalogue::Drawn<Declared>>,
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
    let layers = &catalogue.content.layers;
    let icons = match icons(catalogue) {
        Ok((icons, notes)) => {
            warnings.extend(notes);
            Some(icons)
        }
        Err(error) => {
            warnings.push(format!("Resource icons are unavailable: {error}"));
            None
        }
    };
    let decode = match (&icons, &catalogue.gui_decode) {
        (Some(_), Ok(decode)) => Some(*decode),
        (Some(_), Err(why)) => {
            warnings.push(format!("Resource icons are unavailable: {why}"));
            None
        }
        (None, _) => None,
    };
    if let (Some(icons), Some(decode)) = (&icons, decode) {
        let frames: BTreeSet<u32> = missing
            .iter()
            .filter_map(|(token, ordinal)| icons.frames.of(token, *ordinal).ok().flatten())
            .map(|frame| frame - 1)
            .collect();
        let requests: Vec<_> = frames.iter().map(|frame| (&icons.sprite, *frame)).collect();
        let _ = catalogue::sprite_frames_png(layers, &requests, decode, ICON_EDGE);
    }
    let mut refused = false;
    for (token, ordinal) in missing {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Resource request cancelled",
            ));
        }
        let mut notes = Vec::new();
        let name = resource_name(&catalogue.localisation, token, &mut notes);
        if *ordinal == 1 {
            warnings.extend(notes);
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
                (Ok(Some(frame)), Some(decode)) => match super::artwork::resolve_sprite_type_on_layers(
                    layers,
                    layers.recorded_language().unwrap_or("english"),
                    &icons.sprite,
                    frame - 1,
                    super::artwork::ArtRenderVariant {
                        max_edge: ICON_EDGE,
                        decode,
                    },
                )
                .into_result()
                {
                    Ok(Some(icon)) => Some(icon),
                    Ok(None) => None,
                    Err(error) => {
                        refused = true;
                        warnings.push(format!("Resource icons are unavailable: {error}"));
                        None
                    }
                },
                (Ok(Some(_)), None) => None,
                (Err(message), _) => {
                    warnings.push(message);
                    None
                }
                (Ok(None), _) => {
                    warnings.push(format!(
                        "{token}: the recorded content gives no usable icon frame."
                    ));
                    None
                }
            };
            Declared { name, icon }
        });
        declared.get();
        cached.insert((token.clone(), *ordinal), declared);
    }
    Ok(())
}

pub struct ResourceNaming {
    catalogue: Result<Arc<Catalogue>, String>,
    cache: Arc<std::sync::Mutex<catalogue::ByCatalogue<ContentState>>>,
}

impl ResourceNaming {
    pub fn stored(source: &Source, context: &LoadContext) -> Self {
        Self {
            catalogue: context
                .content()
                .and_then(|content| catalogue::recorded_catalogue(source, content))
                .map_err(|error| error.to_string()),
            cache: source.state(),
        }
    }

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
    state: &std::sync::Mutex<catalogue::ByCatalogue<ContentState>>,
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
                    .cloned()
                    .collect();
                cache.content = Some(ContentCache {
                    catalogue: Arc::clone(catalogue),
                    resources: BTreeMap::new(),
                    warnings: standing,
                });
            }
            let content = cache.content.as_mut().expect("content cached");
            load_resource_content(
                catalogue,
                &declarations,
                &mut content.resources,
                &mut content.warnings,
                cancelled,
            )?;
            warnings.extend(content.warnings.iter().cloned());
            let infos: Vec<ResourceInfo> = declarations
                .iter()
                .enumerate()
                .map(|(index, key)| {
                    info(index, content.resources.get(key).map(catalogue::Drawn::get))
                })
                .collect();
            if infos.iter().any(|resource| resource.icon.is_some()) {
                warnings.push(crate::products::artwork::CURRENT_INSTALL_ART_NOTICE.to_owned());
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

    #[test]
    fn resource_name_distinguishes_absent_key_from_scripted_expression() {
        let loc: Localisation = [
            ("PRODUCTION_MATERIALS_ALPHA".to_owned(), "Alpha Ore".to_owned()),
            ("PRODUCTION_MATERIALS_BETA".to_owned(), "[Root.GetName] Alloy".to_owned()),
        ]
        .into_iter()
        .collect();
        let mut warnings = Vec::new();
        assert_eq!(resource_name(&loc, "alpha", &mut warnings), Some("Alpha Ore".to_owned()));
        assert!(warnings.is_empty(), "{warnings:?}");

        let mut missing_warnings = Vec::new();
        assert_eq!(resource_name(&loc, "gamma", &mut missing_warnings), None);
        assert!(
            missing_warnings.iter().any(|w| w.contains("PRODUCTION_MATERIALS_GAMMA") && w.contains("missing")),
            "{missing_warnings:?}"
        );

        let mut scripted_warnings = Vec::new();
        assert_eq!(resource_name(&loc, "beta", &mut scripted_warnings), None);
        assert!(
            scripted_warnings.iter().any(|w| w.contains("PRODUCTION_MATERIALS_BETA") && w.contains("[Root.GetName]")),
            "{scripted_warnings:?}"
        );
    }
}
