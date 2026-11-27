
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::Source;
use content_reader_layers::content::ResolvedContent;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct SeaContent {
    pub sea: Result<BTreeSet<u32>, String>,
    pub land: Result<BTreeSet<u32>, String>,
    pub all: BTreeSet<u32>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StrategicRegionEntry {
    pub id: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub land: Option<bool>,
    pub sea: Option<bool>,
    pub membership_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StrategicRegionsReply {
    pub regions: Vec<StrategicRegionEntry>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StrategicRegionsReply);

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
        let kinds = (|| -> io::Result<(BTreeSet<u32>, BTreeSet<u32>)> {
            let files = content_reader_key_names::terrain::read_default_map(layers)?;
            let definitions = content_reader_map::map::read_definitions(&layers.read(&files.file("definitions")?)?)?;
            let mut sea = BTreeSet::new();
            let mut land = BTreeSet::new();
            for province in definitions.provinces.values() {
                match province.kind {
                    content_reader_map::map::ProvinceKind::Sea => {
                        sea.insert(province.id);
                    }
                    content_reader_map::map::ProvinceKind::Land => {
                        land.insert(province.id);
                    }
                    content_reader_map::map::ProvinceKind::Lake => {}
                }
            }
            Ok((sea, land))
        })()
        .map_err(|error| error.to_string());
        if let Err(why) = &kinds {
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
        let all: BTreeSet<u32> = regions.regions.keys().copied().collect();
        let sea = kinds.as_ref().map(|(sea, _)| regions.regions.iter().filter(|(_, region)| region.provinces.iter().any(|province| sea.contains(province))).map(|(&id, _)| id).collect()).map_err(Clone::clone);
        let land = kinds.map(|(_, land)| regions.regions.iter().filter(|(_, region)| region.provinces.iter().any(|province| land.contains(province))).map(|(&id, _)| id).collect());
        let ratio = dashboard_base::catalogue::defines_for_layers(layers).and_then(|defines| defines.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO")).map_err(|error| error.to_string());
        let content = Arc::new(Self { sea, land, all, minima, ratio, notes });
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

pub fn catalogue_entries(places: &super::place_names::Content, sea: &SeaContent) -> Vec<StrategicRegionEntry> {
    let sea_set = sea.sea.as_ref().ok();
    let land_set = sea.land.as_ref().ok();
    sea.all
        .iter()
        .map(|&id| {
            let (name, name_missing) = match places.region(id) {
                Ok(name) => (Some(name), None),
                Err(reason) => (None, Some(reason)),
            };
            StrategicRegionEntry {
                id,
                name,
                name_missing,
                land: land_set.map(|set| set.contains(&id)),
                sea: sea_set.map(|set| set.contains(&id)),
                membership_missing: sea.sea.as_ref().err().or_else(|| sea.land.as_ref().err()).cloned(),
            }
        })
        .collect()
}

pub struct StrategicRegions;

impl Product for StrategicRegions {
    const KEY: &'static str = "strategic-regions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = StrategicRegionsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StrategicRegionEntry::decl(), StrategicRegionsReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let _branch = params.branch();
        let places = super::place_names::Content::read(source, load)?;
        let sea = SeaContent::read(source, load)?;
        let regions = catalogue_entries(&places, &sea);
        let mut warnings = sea.notes.clone();
        warnings.extend(places.warnings());
        Ok(StrategicRegionsReply { regions, warnings })
    }
}
