use dashboard_base::{
    catalogue::{self, KeyedResults, Refusals},
    content,
    prepared::query::progressive,
};
use dashboard_keyed::{countries, query::Query, regions::RegionNames};
use installed_content_localisation::localisation::Localisation;
use replay_data::rows::{TableId, tables};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub use dashboard_keyed::query::Choice;

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "AirOperationTheatreChoice")]
pub struct TheatreChoice {
    pub id: String,
    pub name: String,
    pub regions: Vec<String>,
}

/// A requested country and hour window, as every air operations product takes them.
pub fn validate(
    branch: &str,
    tag: &str,
    first: Option<i32>,
    last: Option<i32>,
) -> Result<(), &'static str> {
    dashboard_keyed::query::validate_window(branch, first, last)?;
    countries::validate(tag)
}

pub use dashboard_keyed::{
    countries::selected,
    query::{empty, finish, params},
};

pub fn parts(name: &str) -> Option<(&str, u32, &str)> {
    let mut parts = name.split('|');
    let country = parts.next()?;
    let region = parts.next()?.parse().ok()?;
    let field = parts.next()?;
    (parts.next().is_none()).then_some((country, region, field))
}

/// Counts observations refused by their named input, including across saved blocks.
pub fn note(notes: &mut BTreeMap<String, u64>, reason: impl Into<String>) {
    *notes.entry(reason.into()).or_default() += 1;
}
pub fn warnings(notes: &BTreeMap<String, u64>) -> Vec<String> {
    dashboard_keyed::query::refusals(notes, "air observations")
}

pub struct Labels {
    pub localisation: Result<Arc<Localisation>, String>,
    regions: Result<Arc<RegionNames>, String>,
    pub warnings: Vec<String>,
}

/// Successes and named failures share the same recorded-content/language binding.
pub fn labels(query: &Query<'_>) -> Result<Arc<Labels>, String> {
    let options = query.content.ok_or("Game folders are not chosen")?;
    let key = content::localisation_key(query.source, options).map_err(|error| error.to_string())?;
    query
        .source
        .state::<KeyedResults<content::LocalisationKey, Labels>>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_init(&key, |_| {
            let mut warnings = Vec::new();
            content::check_replay(
                query.source.replay_ref(),
                options.allow_unverified,
                &mut warnings,
            )?;
            let localisation =
                content::localisation(query.source, options).map_err(|e| e.to_string());
            let regions = RegionNames::read(query);
            if let Ok(names) = &regions {
                warnings.extend(names.warnings.clone());
            }
            Ok(Labels {
                localisation,
                regions,
                warnings,
            })
        })
        .map_err(|error| error.to_string())
}

pub fn localised(loc: &Localisation, key: &str) -> Result<String, String> {
    if !key.is_empty() && loc.get(key).is_none() {
        return Err(format!(
            "Localisation {key} is absent from the recorded content stack"
        ));
    }
    catalogue::localised(loc, key, catalogue::View::Plain)
}

pub fn region_choices(
    ids: &BTreeSet<u32>,
    labels: &Result<Arc<Labels>, String>,
    notes: &mut Refusals,
) -> Vec<Choice> {
    let names = labels
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|labels| labels.regions.clone());
    dashboard_keyed::regions::choices(&names, ids, notes)
}

/// The note shown when stored names take the recording's language, or why they are refused
/// ([`content::stored_name_language`]).
pub fn language_note(query: &Query<'_>) -> Option<String> {
    content::stored_name_language(query.source, query.content).1
}

pub fn stored(
    query: &Query<'_>,
    labels: &Result<Arc<Labels>, String>,
    bytes: &[u8],
    what: &str,
) -> Result<String, String> {
    let language = &query.source.replay_ref().identity.language;
    let recorded = language.strip_prefix("l_").unwrap_or(language);
    // Stored names need the language guard, not a successful content database.
    let mut fallback = Localisation::default();
    fallback.language = content::stored_name_language(query.source, query.content).0;
    let loc = labels
        .as_ref()
        .ok()
        .and_then(|labels| labels.localisation.as_ref().ok())
        .map_or(&fallback, Arc::as_ref);
    let text = dashboard_text::scripted_text::stored_text(loc, recorded, bytes, what)?;
    catalogue::plain_text(&text)
}

/// Theatre choices are a reached-hour projection, never an undated union of a
/// changing history. Call only for an explicit detail hour or a one-hour window.
pub fn theatres(
    query: &Query<'_>,
    tag: &str,
    hour: i32,
    labels: &Result<Arc<Labels>, String>,
    notes: &mut Refusals,
) -> io::Result<Option<Vec<TheatreChoice>>> {
    let tables = &[
        TableId::AirTheatres,
        TableId::AirTheatreGroups,
        TableId::AirTheatreMembers,
        TableId::AirWings,
        TableId::Countries,
        TableId::CountryLinks,
    ];
    if query.through().is_some_and(|last| hour > last) {
        return Ok(None);
    }
    let Some(frame) =
        progressive::frame_at(query.source, query.branch, hour, tables, query.cancelled)?
    else {
        notes.note(
            "Air-theatre membership is not recorded at the requested hour",
            hour.to_string(),
        );
        return Ok(None);
    };
    let result = (|| -> Result<_, String> {
        let countries = countries::Countries::read(&frame)
            .ok_or("Theatre country identities are not recorded")?;
        let theatres = frame
            .complete::<tables::AirTheatres>()
            .map_err(|e| e.to_string())?;
        let mut memberships = crate::theatres::regions(&frame)?;
        let mut ordered: Vec<_> = theatres.iter().collect();
        ordered.sort_by_key(|theatre| theatre.ordinal);
        let mut choices = Vec::new();
        for theatre in ordered {
            let owner = countries
                .raw(theatre.owner)
                .ok_or("Air-theatre owner is unresolved")?;
            if !selected(&owner, tag) {
                continue;
            }
            let name =
                stored(query, labels, &theatre.name, "Air theatre name").unwrap_or_else(|why| {
                    notes.note(why, format!("theatre {}", theatre.id));
                    "Air theatre name unavailable".into()
                });
            choices.push(TheatreChoice {
                id: theatre.id.to_string(),
                name,
                regions: memberships
                    .remove(&theatre.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|region| region.to_string())
                    .collect(),
            });
        }
        Ok(choices)
    })();
    match result {
        Ok(choices) => Ok(Some(choices)),
        Err(why) => {
            notes.note(why, "air-theatre membership");
            Ok(None)
        }
    }
}
