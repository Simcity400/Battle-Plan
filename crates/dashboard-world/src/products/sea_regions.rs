
use dashboard_base::products::LoadContext;
use dashboard_base::Source;
use content_reader_layers::content::ResolvedContent;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct SeaContent {
    pub sea: Result<BTreeSet<u32>, String>,
    pub minima: BTreeMap<u32, Result<i64, String>>,
    pub ratio: Result<i64, String>,
    pub notes: Vec<String>,
}

#[derive(Default)]
struct Cached {
    held: Option<(Arc<ResolvedContent>, Arc<SeaContent>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[cfg_attr(any(test, feature = "export-types"), ts(rename = "SeaRegionChoice"))]
pub struct Choice {
    pub id: String,
    pub name: String,
}

impl SeaContent {
    pub fn read(source: &Source, load: &LoadContext) -> io::Result<Arc<Self>> {
        let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Sea regions need the recorded content stack"))?;
        let resolved = source.content(&options.options)?;
        let state = source.state::<Cached>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((held, content)) = &state.held
            && Arc::ptr_eq(held, &resolved)
        {
            return Ok(Arc::clone(content));
        }
        let layers = &resolved.layers;
        let regions = dashboard_base::content::strategic_regions(layers)?;
        let mut notes = regions.notes.clone();
        let sea = (|| -> io::Result<BTreeSet<u32>> {
            let files = content_reader_key_names::terrain::read_default_map(layers)?;
            let definitions = content_reader_map::map::read_definitions(&layers.read(&files.file("definitions")?)?)?;
            Ok(definitions.provinces.values().filter(|province| province.kind == content_reader_map::map::ProvinceKind::Sea).map(|province| province.id).collect())
        })()
        .map_err(|error| error.to_string());
        if let Err(why) = &sea {
            notes.push(format!("1 naval region membership unavailable: {why}"));
        }
        let terrain = content_reader_key_names::terrain::read_definitions(layers).map_err(|error| error.to_string());
        if let Ok(terrain) = &terrain {
            notes.extend(terrain.skipped.iter().map(|why| format!("1 naval terrain content refusal: {why}")));
        }
        let minima = regions
            .regions
            .iter()
            .map(|(&id, region)| {
                let minimum = terrain.as_ref().map_err(Clone::clone).and_then(|terrain| {
                    let at = region
                        .naval_terrain
                        .as_ref()
                        .and_then(|name| terrain.category(name.as_bytes()))
                        .or_else(|| terrain.category(b"ocean"))
                        .ok_or("Naval terrain and ocean fallback are absent from recorded content")?;
                    Ok(terrain.categories[at].minimum_seazone_dominance)
                });
                (id, minimum)
            })
            .collect();
        let sea = sea.map(|sea| regions.regions.iter().filter(|(_, region)| region.provinces.iter().any(|province| sea.contains(province))).map(|(&id, _)| id).collect());
        let ratio = content_reader_defines::defines::read(layers).and_then(|defines| defines.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO")).map_err(|error| error.to_string());
        let content = Arc::new(Self { sea, minima, ratio, notes });
        state.held = Some((resolved, Arc::clone(&content)));
        Ok(content)
    }

    pub fn minimum(&self, id: u32) -> Result<i64, String> {
        self.minima.get(&id).cloned().unwrap_or_else(|| Err(format!("Naval terrain minimum for region {id} is unavailable")))
    }
}

pub fn choices(places: &super::place_names::Content, sea: &SeaContent, ids: impl IntoIterator<Item = u32>, notes: &mut dashboard_base::catalogue::Refusals) -> Vec<Choice> {
    let mut all: BTreeSet<u32> = ids.into_iter().collect();
    if let Ok(sea) = &sea.sea {
        all.extend(sea);
    }
    all.into_iter()
        .map(|id| Choice {
            id: id.to_string(),
            name: places.region(id).unwrap_or_else(|reason| {
                notes.note(reason, id.to_string());
                format!("Region {id} (name unavailable)")
            }),
        })
        .collect()
}
