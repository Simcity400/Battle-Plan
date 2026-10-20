use dashboard_base::{
    catalogue::{CachedResult, KeyedResults, Refusals},
    content,
};
use dashboard_keyed::{
    countries,
    query::{KeyedWindow, Query},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub const SCALE: i32 = 100_000;
pub use dashboard_keyed::query::Choice;

pub use dashboard_keyed::{
    countries::{selected, validate as tag},
    query::{empty, finish, params, validate_window as validate},
};

pub fn note(notes: &mut BTreeMap<String, u64>, reason: impl Into<String>) {
    *notes.entry(reason.into()).or_default() += 1;
}
pub fn warnings(notes: &BTreeMap<String, u64>) -> Vec<String> {
    dashboard_keyed::query::refusals(notes, "naval observations")
}
pub fn types() -> Vec<String> {
    vec![
        Choice::decl(),
        KeyedWindow::decl(),
        dashboard_keyed::query::KeyedSeries::decl(),
        dashboard_base::products::country_groups::HourSpan::decl(),
    ]
}

pub struct Labels {
    pub regions: Result<Arc<dashboard_keyed::regions::RegionNames>, String>,
    pub sea_regions: Result<BTreeSet<u32>, String>,
    pub minima: BTreeMap<u32, Result<i64, String>>,
    pub ratio: Result<i64, String>,
    pub warnings: Vec<String>,
}
pub fn labels(q: &Query<'_>) -> Result<Arc<Labels>, String> {
    let options = q.content.ok_or("Game folders are not chosen")?;
    let key = content::localisation_key(options).map_err(|e| e.to_string())?;
    q.source
        .state::<KeyedResults<content::LocalisationKey, Labels>>()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_init(&key, |_| {
            let mut warnings = Vec::new();
            content::check_replay(
                q.source.replay_ref(),
                options.allow_unverified,
                &mut warnings,
            )?;
            let resolved = q.source.content(&options.options)?;
            let layers = &resolved.layers;
            let definitions = layers
                .derived::<CachedResult<installed_content_regions::regions::StrategicRegions>>()
                .get_or_init(|| installed_content_regions::regions::read(layers))?;
            warnings.extend(definitions.notes.clone());
            let sea = (|| -> io::Result<BTreeSet<u32>> {
                let files = installed_content_core::terrain::read_default_map(layers)?;
                Ok(installed_content_core::map::read_definitions(
                    &layers.read(&files.file("definitions")?)?,
                )?
                .provinces
                .values()
                .filter(|p| p.kind == installed_content_core::map::ProvinceKind::Sea)
                .map(|p| p.id)
                .collect())
            })()
            .map_err(|e| e.to_string());
            let terrain =
                installed_content_core::terrain::read_definitions(layers).map_err(|e| e.to_string());
            let ratio = installed_content_core::defines::read(layers)
                .and_then(|d| d.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO"))
                .map_err(|e| e.to_string());
            let mut minima = BTreeMap::new();
            if let Err(why) = &sea {
                warnings.push(format!("1 naval region membership unavailable: {why}"));
            }
            for (&id, region) in &definitions.regions {
                let minimum = terrain.as_ref().map_err(Clone::clone).and_then(|t| {
                    let at = region
                        .naval_terrain
                        .as_ref()
                        .and_then(|name| t.category(name.as_bytes()))
                        .or_else(|| t.category(b"ocean"))
                        .ok_or(
                            "Naval terrain and ocean fallback are absent from recorded content",
                        )?;
                    Ok(t.categories[at].minimum_seazone_dominance)
                });
                minima.insert(id, minimum);
            }
            if let Ok(t) = &terrain {
                warnings.extend(
                    t.skipped
                        .iter()
                        .map(|why| format!("1 naval terrain content refusal: {why}")),
                );
            }
            let sea_regions = sea.map(|sea| {
                definitions
                    .regions
                    .iter()
                    .filter(|(_, r)| r.provinces.iter().any(|p| sea.contains(p)))
                    .map(|(&id, _)| id)
                    .collect()
            });
            let regions = dashboard_keyed::regions::RegionNames::read(q);
            Ok(Labels {
                regions,
                sea_regions,
                minima,
                ratio,
                warnings,
            })
        })
        .map_err(|e| e.to_string())
}
pub fn choices(
    ids: &BTreeSet<u32>,
    labels: &Result<Arc<Labels>, String>,
    notes: &mut Refusals,
) -> Vec<Choice> {
    let names = labels.as_ref().map_err(Clone::clone).and_then(|labels| labels.regions.clone());
    dashboard_keyed::regions::choices(&names, ids, notes)
}

/// Actual reserved-country identity is independent of the positive roster.
pub fn country_part(
    frame: &replay_frame::frame::DecodedFrame,
    countries: Option<&countries::Countries>,
    raw: u32,
) -> Option<String> {
    if raw == 0 || countries.and_then(|c| c.slot(raw)) == Some(0) {
        let [row] = frame
            .complete::<replay_data::rows::tables::ReservedCountryNaming>()
            .ok()?
        else {
            return None;
        };
        let tag = std::str::from_utf8(&row.tag).ok()?;
        return (!tag.contains('|')).then(|| countries::part(tag, 0));
    }
    countries.and_then(|c| c.raw(raw))
}
pub fn viewer_slots(
    frame: &replay_frame::frame::DecodedFrame,
    countries: Option<&countries::Countries>,
    tag: &str,
) -> BTreeSet<u32> {
    let mut slots: BTreeSet<_> = frame
        .complete::<replay_data::rows::tables::Countries>()
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            countries
                .and_then(|c| c.slotted(r.canonical_slot))
                .filter(|c| selected(c, tag))
                .map(|_| r.canonical_slot)
        })
        .collect();
    if country_part(frame, countries, 0).is_some_and(|c| selected(&c, tag)) {
        slots.insert(0);
    }
    slots
}
