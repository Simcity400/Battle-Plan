use content_reader_mesh::mesh::MeshProducts as _;

use replay_frame::frame::DecodedFrame;
use dashboard_base::{Source, catalogue::{self, CachedResult, Catalogue, View}, products::{ContentOptions, LoadContext, Product, check_reply_budget}};
use content_reader_equipment_graphics::{equipment_graphics, equipment_graphics::aircraft};
use content_reader_layers::{LayerRole, ResolvedLayers};
use replay_data::{rows::TableId, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AircraftCatalogueParams {
    #[doc = " Matched without case against names, titles and what references them;"]
    #[doc = " empty matches every name."]
    pub query: String,
    #[doc = " The first matching entry to answer."]
    pub first: u32,
    #[doc = " How many matching entries to answer from `first`."]
    pub count: u32,
    #[doc = " Only the base game's entries, with its DLC's, or only one mod's; absent"]
    #[doc = " matches every origin."]
    #[serde(default)]
    pub origin: Option<AircraftOriginFilter>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AircraftOriginFilter {
    Game,
    Mod(String),
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AircraftOrigin {
    Game { dlc: Option<String> },
    Mod { id: String, name: String },
}

impl AircraftOrigin {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Game { .. }, Self::Game { .. }) => true,
            (Self::Mod { id, .. }, Self::Mod { id: other, .. }) => id == other,
            _ => false,
        }
    }

    fn chosen(&self, filter: &AircraftOriginFilter) -> bool {
        match (self, filter) {
            (Self::Game { .. }, AircraftOriginFilter::Game) => true,
            (Self::Mod { id, .. }, AircraftOriginFilter::Mod(wanted)) => id == wanted,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AircraftOriginCount {
    pub origin: AircraftOrigin,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AircraftKind {
    Entity,
    Mesh,
    Unregistered,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AircraftReference {
    Pool {
        bucket: String,
        equipment: String,
        file: String,
    },
    Ladder {
        key: String,
        prefix: Option<String>,
    },
    Design {
        equipment: String,
    },
    CloneOf {
        of: String,
    },
    ClonedBy {
        by: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AircraftRefusalKind {
    Unregistered,
    Declaration,
    NoGeometry,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AircraftRefusal {
    pub kind: AircraftRefusalKind,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AircraftEntry {
    #[doc = " The entity or mesh name the map flies, exactly as registered."]
    pub name: String,
    pub kind: AircraftKind,
    #[doc = " The localised names of the equipment that references it, in reference"]
    #[doc = " order without repeats; empty when none has one."]
    pub titles: Vec<String>,
    #[doc = " Why it is an aircraft's."]
    pub references: Vec<AircraftReference>,
    #[doc = " Why it cannot be flown."]
    pub refusal: Option<AircraftRefusal>,
    #[doc = " Who supplies the files it is declared in and drawn from: a mod when any"]
    #[doc = " of them is a mod's; null when no descriptor registers it."]
    pub origin: Option<AircraftOrigin>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AircraftRefusalCount {
    pub kind: AircraftRefusalKind,
    pub label: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AircraftCatalogueReply {
    #[doc = " How many aircraft match the query and origin."]
    pub matching: u32,
    #[doc = " How many aircraft match the query, whatever their origin."]
    pub found: u32,
    pub first: u32,
    #[doc = " Matching entries from `first`: flyable ones first, then refused ones,"]
    #[doc = " each without case by the first title, else the readable name."]
    pub entries: Vec<AircraftEntry>,
    #[doc = " The matching entries that cannot be flown, by kind of reason."]
    pub refusals: Vec<AircraftRefusalCount>,
    #[doc = " Every origin with an aircraft, the base game first and then mods in load"]
    #[doc = " order, each with how many of its aircraft match the query."]
    pub origins: Vec<AircraftOriginCount>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AircraftCatalogueReply);

pub struct AircraftCatalogue;

impl Product for AircraftCatalogue {
    const KEY: &'static str = "aircraft-catalogue";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AircraftCatalogueParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![query, first, count, origin];
    type Reply = AircraftCatalogueReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.count == 0 {
            return Err("An aircraft catalogue page needs at least one entry");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            Self::Params::decl(),
            Self::Reply::decl(),
            AircraftKind::decl(),
            AircraftReference::decl(),
            AircraftRefusalKind::decl(),
            AircraftRefusal::decl(),
            AircraftEntry::decl(),
            AircraftRefusalCount::decl(),
            AircraftOriginFilter::decl(),
            AircraftOrigin::decl(),
            AircraftOriginCount::decl(),
        ]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let built = built(source, context.content()?, cancelled)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let reply = built.page(params);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

struct Listed {
    entry: AircraftEntry,
    search: String,
}

impl Listed {
    fn is_from(&self, origin: &AircraftOrigin) -> bool {
        self.entry
            .origin
            .as_ref()
            .is_some_and(|known| known.same(origin))
    }
}

pub struct Built {
    entries: Vec<Listed>,
    origins: Vec<AircraftOrigin>,
    warnings: Vec<String>,
}

type Cached = Option<(catalogue::Binding, Arc<CachedResult<Built>>)>;

fn built(
    source: &Source,
    options: &ContentOptions,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Built>> {
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state
        .as_ref()
        .is_some_and(|(binding, _)| catalogue::binding_matches(binding, options))
    {
        *state = Some((
            catalogue::binding(options),
            Arc::new(CachedResult::default()),
        ));
    }
    let result = Arc::clone(&state.as_ref().expect("catalogue binding retained").1);
    drop(state);
    result.get_or_init(|| {
        let catalogue = catalogue::catalogue(source, options)?;
        let designs = recorded_designs(source, cancelled)?;
        let mods: BTreeMap<String, String> = source
            .replay()
            .identity
            .mods
            .iter()
            .map(|recorded| (recorded.id.clone(), recorded.display_name.clone()))
            .collect();
        Ok(Built::new(&catalogue, &designs, &mods))
    })
}

pub struct Content {
    pub roster: Arc<content_reader_mesh::mesh::Roster>,
    pub equipment: Arc<content_reader_key_names::equipment::Database>,
    pub pools: equipment_graphics::Content,
    pub warnings: Vec<String>,
}

impl Content {
    pub fn read(catalogue: &Arc<Catalogue>) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let roster = layers.descriptor_roster()?;
        let mut warnings = Vec::new();
        let equipment = dashboard_base::catalogue::equipment_units(catalogue).unwrap_or_else(|why| {
            warnings.push(format!(
                "Which names are aircraft cannot be told from equipment: the equipment database is refused ({why}); every entity and mesh is still listed"
            ));
            Arc::new(content_reader_key_names::equipment::Database::default())
        });
        let pools = (|| {
            let default_map = content_reader_key_names::terrain::read_default_map(layers)?;
            let names = equipment_graphics::Names::read(layers, &default_map, &mut warnings)?;
            equipment_graphics::Content::load(layers, &names)
        })()
        .unwrap_or_else(|why| {
            warnings.push(format!(
                "Graphics pools are refused ({why}), so no pool makes a name an aircraft's"
            ));
            equipment_graphics::Content::default()
        });
        Ok(Self {
            roster,
            equipment,
            pools,
            warnings,
        })
    }

    pub fn catalogue(&self, designs: &[(String, String)]) -> aircraft::Catalogue {
        aircraft::Catalogue::build(&aircraft::Inputs {
            roster: &self.roster,
            equipment: &self.equipment,
            pools: &self.pools,
            designs,
        })
    }
}

fn recorded_designs(source: &Source, cancelled: &AtomicBool) -> io::Result<Vec<(String, String)>> {
    let replay = source.replay();
    let selected = [TableId::VariantDefinitions, TableId::LexicalTokens];
    let mut designs = BTreeSet::new();
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Aircraft catalogue cancelled");
    dashboard_core::blocks::scan_blocks(
        &replay,
        replay.index.iter(),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
                        break;
                    };
                    if !dashboard_text::metadata::slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::VariantDefinitions,
                        decoded.hour,
                    ) {
                        continue;
                    }
                    let (Ok(variants), Ok(lexical)) = (
                        decoded.complete::<tables::VariantDefinitions>(),
                        decoded.complete::<tables::LexicalTokens>(),
                    ) else {
                        continue;
                    };
                    for variant in variants
                        .iter()
                        .filter(|row| !row.explicit_entity.is_empty())
                    {
                        let Ok(model) = content_reader_parser::parser::text(&variant.explicit_entity)
                        else {
                            continue;
                        };
                        let kind = variant_token(variant, lexical);
                        designs.insert((kind, model.to_owned()));
                    }
                }
            }
            Ok(())
        },
    )?;
    Ok(designs.into_iter().collect())
}

impl Built {
    pub fn new(
        catalogue: &Arc<Catalogue>,
        designs: &[(String, String)],
        mods: &BTreeMap<String, String>,
    ) -> Self {
        match Content::read(catalogue) {
            Ok(content) => Self::from_content(catalogue, &content, designs, mods),
            Err(why) => Self {
                entries: Vec::new(),
                origins: Vec::new(),
                warnings: vec![format!("The descriptor roster is refused: {why}")],
            },
        }
    }

    pub fn from_content(
        catalogue: &Catalogue,
        content: &Content,
        designs: &[(String, String)],
        mods: &BTreeMap<String, String>,
    ) -> Self {
        let listing = content.catalogue(designs);
        let equipment = &content.equipment;
        let spelled = |folded: &str| {
            equipment
                .named(folded)
                .and_then(|kind| kind.name_str().ok().map(str::to_owned))
                .unwrap_or_else(|| folded.to_owned())
        };
        let title = |key: &str| {
            catalogue
                .localisation
                .get(key)
                .is_some()
                .then(|| catalogue.localised(key, View::Plain))
                .flatten()
                .filter(|text| !text.trim().is_empty())
        };
        let suppliers = Suppliers::new(&catalogue.content.layers, mods);
        let descriptors = &content.roster.descriptors;
        let mut entries: Vec<Listed> = listing
            .entries
            .into_iter()
            .filter(aircraft::Entry::aircraft)
            .map(|entry| {
                let mut titles: Vec<String> = Vec::new();
                let mut add = |text: Option<String>| {
                    if let Some(text) = text
                        && !titles.contains(&text)
                    {
                        titles.push(text);
                    }
                };
                for reference in &entry.references {
                    match reference {
                        aircraft::Reference::Pool { equipment, .. } => {
                            add(title(&spelled(equipment)))
                        }
                        aircraft::Reference::Ladder { key, prefix } => add(prefix
                            .as_ref()
                            .and_then(|prefix| title(&format!("{prefix}_{key}")))
                            .or_else(|| title(key))),
                        aircraft::Reference::Design { equipment } => add(title(equipment)),
                        aircraft::Reference::CloneOf { .. }
                        | aircraft::Reference::ClonedBy { .. } => {}
                    }
                }
                let files: Vec<&str> = match descriptors.inherit(&entry.name) {
                    Some(entity) => entity
                        .sources
                        .first()
                        .copied()
                        .and_then(|path| path.to_str())
                        .into_iter()
                        .chain(
                            entity
                                .mesh
                                .and_then(|mesh| descriptors.meshes.get(mesh))
                                .map(|mesh| mesh.file.as_str()),
                        )
                        .collect(),
                    None => descriptors
                        .meshes
                        .get(&entry.name)
                        .map(|mesh| mesh.file.as_str())
                        .into_iter()
                        .collect(),
                };
                let entry = AircraftEntry {
                    kind: match entry.kind {
                        aircraft::Kind::Entity => AircraftKind::Entity,
                        aircraft::Kind::Mesh => AircraftKind::Mesh,
                        aircraft::Kind::Unregistered => AircraftKind::Unregistered,
                    },
                    titles,
                    references: entry.references.into_iter().map(reference).collect(),
                    refusal: entry.refusal.map(|refusal| AircraftRefusal {
                        kind: refusal_kind(refusal.kind),
                        reason: refusal.reason,
                    }),
                    origin: suppliers.origin(&files),
                    name: entry.name,
                };
                Listed {
                    search: search_text(&entry),
                    entry,
                }
            })
            .collect();
        entries.sort_by_cached_key(|listed| {
            let entry = &listed.entry;
            let shown = entry
                .titles
                .first()
                .cloned()
                .unwrap_or_else(|| readable_name(&entry.name));
            (entry.refusal.is_some(), shown.to_lowercase(), entry.name.clone())
        });
        let origins = suppliers
            .listed()
            .filter(|origin| entries.iter().any(|listed| listed.is_from(origin)))
            .collect();
        let mut warnings = content.warnings.clone();
        warnings.extend(listing.diagnostics);
        Self {
            entries,
            origins,
            warnings,
        }
    }

    pub fn page(&self, params: &AircraftCatalogueParams) -> AircraftCatalogueReply {
        let query = params.query.trim().to_lowercase();
        let found: Vec<&Listed> = self
            .entries
            .iter()
            .filter(|listed| query.is_empty() || listed.search.contains(&query))
            .collect();
        let total = count(found.len());
        let origins = self
            .origins
            .iter()
            .map(|origin| AircraftOriginCount {
                origin: origin.clone(),
                count: count(found.iter().filter(|listed| listed.is_from(origin)).count()),
            })
            .collect();
        let matching: Vec<&Listed> = found
            .into_iter()
            .filter(|listed| {
                params.origin.as_ref().is_none_or(|filter| {
                    listed
                        .entry
                        .origin
                        .as_ref()
                        .is_some_and(|known| known.chosen(filter))
                })
            })
            .collect();
        let mut refusals = BTreeMap::new();
        for refusal in matching
            .iter()
            .filter_map(|listed| listed.entry.refusal.as_ref())
        {
            *refusals.entry(refusal.kind).or_insert(0_u32) += 1;
        }
        AircraftCatalogueReply {
            matching: count(matching.len()),
            found: total,
            first: params.first,
            entries: matching
                .iter()
                .skip(params.first as usize)
                .take(params.count as usize)
                .map(|listed| listed.entry.clone())
                .collect(),
            refusals: refusals
                .into_iter()
                .map(|(kind, count)| AircraftRefusalCount {
                    kind,
                    label: refusal_label(kind).to_owned(),
                    count,
                })
                .collect(),
            origins,
            warnings: self.warnings.clone(),
        }
    }
}

struct Suppliers<'a> {
    layers: &'a ResolvedLayers,
    origins: Vec<AircraftOrigin>,
}

impl<'a> Suppliers<'a> {
    fn new(layers: &'a ResolvedLayers, mods: &BTreeMap<String, String>) -> Self {
        let origins = layers
            .layers
            .iter()
            .map(|layer| {
                let named = |name: &str| AircraftOrigin::Mod {
                    id: layer.id.clone(),
                    name: name.to_owned(),
                };
                if let Some(name) = layer.visual_mod_name() {
                    return named(name);
                }
                match layer.role {
                    LayerRole::Vanilla | LayerRole::Builtin | LayerRole::AboveMods => {
                        AircraftOrigin::Game { dlc: None }
                    }
                    LayerRole::Dlc => AircraftOrigin::Game {
                        dlc: Some(
                            layer
                                .dlc_name()
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| layer.id.clone()),
                        ),
                    },
                    LayerRole::Mod | LayerRole::Visual => {
                        named(mods.get(&layer.id).map_or(layer.id.as_str(), String::as_str))
                    }
                    LayerRole::UserDirectory => named("User folder"),
                }
            })
            .collect();
        Self { layers, origins }
    }

    fn origin(&self, files: &[&str]) -> Option<AircraftOrigin> {
        let found: Vec<&AircraftOrigin> = files
            .iter()
            .filter_map(|file| self.origins.get(self.layers.layer_index(file)?))
            .collect();
        let pick = |wanted: fn(&AircraftOrigin) -> bool| found.iter().copied().find(|origin| wanted(origin));
        pick(|origin| matches!(origin, AircraftOrigin::Mod { .. }))
            .or_else(|| pick(|origin| matches!(origin, AircraftOrigin::Game { dlc: Some(_) })))
            .or_else(|| found.first().copied())
            .cloned()
    }

    fn listed(&self) -> impl Iterator<Item = AircraftOrigin> + '_ {
        let game = self
            .origins
            .iter()
            .any(|origin| matches!(origin, AircraftOrigin::Game { .. }))
            .then_some(AircraftOrigin::Game { dlc: None });
        let mods = self.origins.iter().enumerate().filter_map(|(at, origin)| {
            let first = self.origins[..at].iter().all(|earlier| !earlier.same(origin));
            (matches!(origin, AircraftOrigin::Mod { .. }) && first).then(|| origin.clone())
        });
        game.into_iter().chain(mods)
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn reference(reference: aircraft::Reference) -> AircraftReference {
    match reference {
        aircraft::Reference::Pool {
            bucket,
            equipment,
            file,
        } => AircraftReference::Pool {
            bucket,
            equipment,
            file,
        },
        aircraft::Reference::Ladder { key, prefix } => AircraftReference::Ladder { key, prefix },
        aircraft::Reference::Design { equipment } => AircraftReference::Design { equipment },
        aircraft::Reference::CloneOf { of } => AircraftReference::CloneOf { of },
        aircraft::Reference::ClonedBy { by } => AircraftReference::ClonedBy { by },
    }
}

fn refusal_kind(kind: aircraft::RefusalKind) -> AircraftRefusalKind {
    match kind {
        aircraft::RefusalKind::Unregistered => AircraftRefusalKind::Unregistered,
        aircraft::RefusalKind::Declaration => AircraftRefusalKind::Declaration,
        aircraft::RefusalKind::NoGeometry => AircraftRefusalKind::NoGeometry,
    }
}

fn refusal_label(kind: AircraftRefusalKind) -> &'static str {
    let kind = match kind {
        AircraftRefusalKind::Unregistered => aircraft::RefusalKind::Unregistered,
        AircraftRefusalKind::Declaration => aircraft::RefusalKind::Declaration,
        AircraftRefusalKind::NoGeometry => aircraft::RefusalKind::NoGeometry,
    };
    kind.label()
}

fn readable_name(name: &str) -> String {
    let mut base = name;
    while let Some(len) = ["_entity", "-entity", "_mesh", "-mesh"]
        .iter()
        .find(|suffix| base.to_ascii_lowercase().ends_with(*suffix))
        .map(|suffix| base.len() - suffix.len())
    {
        base = &base[..len];
    }
    base.split(|c: char| matches!(c, '_' | '-' | '.') || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn search_text(entry: &AircraftEntry) -> String {
    let mut text = entry.name.clone();
    text.push('\n');
    text.push_str(&readable_name(&entry.name));
    text.push('\n');
    text.push_str(&entry.titles.join(" / "));
    for reference in &entry.references {
        let parts: &[&str] = match reference {
            AircraftReference::Pool {
                bucket, equipment, ..
            } => &[bucket, equipment],
            AircraftReference::Ladder { key, prefix } => {
                &[key, prefix.as_deref().unwrap_or_default()]
            }
            AircraftReference::Design { equipment } => &[equipment],
            AircraftReference::CloneOf { of } => &[of],
            AircraftReference::ClonedBy { by } => &[by],
        };
        for part in parts {
            text.push('\n');
            text.push_str(part);
        }
    }
    text.to_lowercase()
}

#[cfg(test)]
mod tests;

fn variant_token(definition: &replay_data::rows::VariantDefinitionRow, lexical: &[replay_data::rows::LexicalTokenRow]) -> String {
    lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .and_then(|index| std::str::from_utf8(&lexical[index].literal).ok().map(str::to_owned))
        .unwrap_or_default()
}
