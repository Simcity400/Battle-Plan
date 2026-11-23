
use super::Art;
use dashboard_base::catalogue::{self, Catalogue};
use dashboard_text::{metadata, scripted_text::ScriptedText};
use dashboard_text::faction_names;
use content_reader_politics::faction_goals::{Catalog, Definition};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
use std::{cell::OnceCell, collections::BTreeMap};
use replay_data::rows::TableId;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionProgressBand {
    pub name: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalDefinition {
    #[doc = " The goal's token as the content names it; none when the recording names none."]
    pub token: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    #[doc = " The native term token (`short_term`, ...), a group's identity."]
    pub category: Option<String>,
    #[doc = " The term's name as the game's interface names it; none when the content does not."]
    pub category_name: Option<String>,
    pub bands: Vec<FactionProgressBand>,
    #[doc = " Why any of the texts or the artwork is unavailable."]
    pub reason: Option<String>,
}

pub(super) struct Texts<'a> {
    frame: &'a DecodedFrame,
    faction: u64,
    cat: &'a Catalogue,
    definitions: Result<std::sync::Arc<Catalog>, String>,
    art: &'a Art,
    lexical: faction_names::LexicalNames<'a>,
    viewpoint: Option<&'a dashboard_base::products::Country>,
    viewer_text: OnceCell<Result<ScriptedText<'a>, String>>,
}

pub(super) fn viewer(frame: &DecodedFrame, viewpoint: Option<&dashboard_base::products::Country>) -> Result<u32, String> {
    if let Some(dashboard_base::products::Country::Slot(slot)) = viewpoint {
        let countries = frame.complete::<tables::Countries>().map_err(|why| why.to_string())?;
        return countries.iter().any(|country| country.canonical_slot == *slot).then_some(*slot).ok_or_else(|| "Faction text's selected country has no country at this hour".to_owned());
    }
    if let Some(dashboard_base::products::Country::Tag(tag)) = viewpoint {
        let countries = frame.complete::<tables::Countries>().map_err(|why| why.to_string())?;
        return countries
            .iter()
            .filter(|country| country.tag.eq_ignore_ascii_case(tag.as_bytes()))
            .map(|country| country.canonical_slot)
            .min()
            .ok_or_else(|| format!("Faction text's selected country {tag} has no country at this hour"));
    }
    let raw = frame
        .complete::<tables::LocalPlayer>()
        .map_err(|why| why.to_string())?
        .first()
        .and_then(|row| row.viewing_country())
        .ok_or("Faction text needs the selected country")?;
    let links = frame.complete::<tables::CountryLinks>().map_err(|why| why.to_string())?;
    metadata::link_slot(links, raw).ok_or_else(|| "Faction text's selected country has no link".to_owned())
}

impl<'a> Texts<'a> {
    pub(super) fn new(frame: &'a DecodedFrame, faction: u64, cat: &'a Catalogue, art: &'a Art, viewpoint: Option<&'a dashboard_base::products::Country>) -> Self {
        Self {
            frame,
            faction,
            cat,
            definitions: content_reader_politics::faction_goals::cached(&cat.content.layers),
            art,
            lexical: faction_names::LexicalNames::from_source(frame),
            viewpoint,
            viewer_text: OnceCell::new(),
        }
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        match &self.definitions {
            Ok(definitions) => definitions.warnings.clone(),
            Err(why) => vec![why.clone()],
        }
    }

    fn scripted<'s>(&'s self, root: Result<u32, String>, held: &'s OnceCell<Result<ScriptedText<'a>, String>>) -> Result<&'s ScriptedText<'a>, String> {
        held.get_or_init(|| {
            let root = root?;
            Ok(ScriptedText::new(self.frame, &self.cat.localisation, self.cat.scripted_content()?, root).viewing(root))
        })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(super) fn localise(&self, key: &str) -> Result<String, String> {
        self.cat.localisation.localise(key.as_bytes()).map_err(|why| why.to_string())
    }

    pub(super) fn expand(&self, text: &str) -> Result<String, String> {
        self.scripted(viewer(self.frame, self.viewpoint), &self.viewer_text)?.expand_faction(text, self.faction)
    }

    pub(super) fn definition(&self, token: Option<u32>) -> FactionGoalDefinition {
        let mut shown = blank();
        let Some(token) = token else { return shown };
        let mut failures = Vec::new();
        let reached = (|| -> Result<&Definition, String> {
            let spelling = faction_names::template_spelling(self.frame, Some(token))?.ok_or("Faction goal token is absent")?;
            shown.token = Some(spelling.clone());
            self.definitions
                .as_ref()
                .map_err(Clone::clone)?
                .get(&spelling)?
                .ok_or_else(|| format!("Faction goal {spelling:?} is absent from recorded content"))
        })();
        let definition = match reached {
            Ok(definition) => definition,
            Err(why) => {
                shown.reason = Some(why);
                return shown;
            }
        };
        match self.art.goal_icon(self.cat, definition.group.as_deref()) {
            Ok(icon) => shown.icon = icon,
            Err(why) => failures.push(why),
        }
        let render = |bound: &content_reader_script::localization_binding::BoundLocalization| -> Result<String, String> {
            if bound.is_empty() {
                return Ok(String::new());
            }
            let text = faction_names::bound_name(bound, &self.cat.localisation, |name| self.lexical.token_of(name))?;
            let text = if text.contains('[') { self.expand(&text)? } else { text };
            catalogue::plain_text(&text)
        };
        match render(&definition.name) {
            Ok(text) => shown.name = Some(text),
            Err(why) => failures.push(why),
        }
        match render(&definition.description) {
            Ok(text) => shown.description = Some(text),
            Err(why) => failures.push(why),
        }
        match definition.category.as_deref().map(|value| std::str::from_utf8(value).map(str::to_owned)).transpose() {
            Ok(category) => shown.category = category,
            Err(_) => failures.push("Faction goal category is not UTF-8".into()),
        }
        shown.category_name = shown.category.as_deref().and_then(|category| term_name(&self.cat.localisation, category));
        shown.bands = bands(definition, &self.cat.localisation);
        shown.reason = (!failures.is_empty()).then(|| failures.join("; "));
        shown
    }
}

fn blank() -> FactionGoalDefinition {
    FactionGoalDefinition {
        token: None,
        name: None,
        description: None,
        icon: None,
        category: None,
        category_name: None,
        bands: Vec::new(),
        reason: None,
    }
}

pub(super) fn unnamed(frame: &DecodedFrame, token: Option<u32>, why: &str) -> FactionGoalDefinition {
    let mut shown = blank();
    let Some(token) = token else { return shown };
    match faction_names::template_spelling(frame, Some(token)) {
        Ok(Some(spelling)) => {
            shown.token = Some(spelling);
            shown.reason = Some(why.to_owned());
        }
        Ok(None) => shown.reason = Some("Faction goal token is absent".into()),
        Err(failure) => shown.reason = Some(failure),
    }
    shown
}

pub(super) fn term_name(localisation: &content_reader_localisation::localisation::Localisation, category: &str) -> Option<String> {
    let key = category.to_ascii_uppercase();
    localisation.get(&key)?;
    catalogue::localised(localisation, &key, catalogue::View::Plain).ok().filter(|name| !name.is_empty())
}

fn bands(definition: &Definition, localisation: &content_reader_localisation::localisation::Localisation) -> Vec<FactionProgressBand> {
    definition
        .progress_sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let number = |kind: &str| {
                section
                    .iter()
                    .rfind(|field| field.keyword_is(kind))
                    .and_then(|field| field.value.scalar())
                    .and_then(|value| std::str::from_utf8(value).ok()?.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
            };
            FactionProgressBand {
                name: definition
                    .progress_section_keys
                    .get(index)
                    .and_then(Option::as_deref)
                    .and_then(|key| std::str::from_utf8(key).ok())
                    .filter(|key| localisation.get(key).is_some())
                    .and_then(|key| catalogue::localised(localisation, key, catalogue::View::Plain).ok())
                    .filter(|name| !name.is_empty()),
                min: number("min"),
                max: number("max"),
            }
        })
        .collect()
}

pub(super) fn counted(definitions: &[FactionGoalDefinition]) -> Vec<String> {
    let mut reasons = BTreeMap::<&str, usize>::new();
    for definition in definitions {
        if let Some(reason) = &definition.reason {
            *reasons.entry(reason).or_default() += 1;
        }
    }
    reasons.into_iter().map(|(reason, count)| format!("{count} faction goal texts: {reason}")).collect()
}

pub(super) fn scripted(frame: &DecodedFrame, faction: u64, cat: &Catalogue) -> bool {
    let Ok(definitions) = content_reader_politics::faction_goals::cached(&cat.content.layers) else { return false };
    let lexical = faction_names::LexicalNames::from_source(frame);
    let spelled = |token: u32| faction_names::template_spelling(frame, Some(token)).ok().flatten();
    let goals = frame
        .complete::<tables::FactionGoalStatus>()
        .ok()
        .into_iter()
        .flat_map(|rows| super::owned(rows, faction, |row| row.faction))
        .filter_map(|row| row.manifest)
        .chain(
            frame
                .complete::<tables::FactionGoalEntries>()
                .ok()
                .into_iter()
                .flat_map(|rows| super::owned(rows, faction, |row| row.faction))
                .filter(|row| row.kind <= 2)
                .filter_map(|row| row.token),
        );
    for token in goals {
        let Some(definition) = spelled(token).and_then(|spelling| definitions.get(&spelling).ok().flatten()) else { continue };
        if [&definition.name, &definition.description].into_iter().any(|bound| {
            !bound.is_empty()
                && faction_names::bound_name(bound, &cat.localisation, |name| lexical.token_of(name)).is_ok_and(|text| text.contains('['))
        }) {
            return true;
        }
    }
    frame.complete::<tables::FactionDoctrine>().is_ok_and(|rows| {
        super::owned(rows, faction, |row| row.faction)
            .iter()
            .filter_map(|row| spelled(row.token))
            .any(|spelling| cat.localisation.localise(spelling.as_bytes()).is_ok_and(|text| text.contains('[')))
    })
}

pub(super) fn text_tables() -> impl Iterator<Item = TableId> {
    dashboard_text::scripted_text::TABLES.iter().copied()
}
