use content_reader_mesh::mesh::MeshProducts as _;

use replay_frame::frame::DecodedFrame;
use dashboard_base::{Source, catalogue::{self, CachedResult, Catalogue, View}, products::{ContentOptions, LoadContext, Product, check_reply_budget}};
use content_reader_equipment_graphics::{equipment_graphics, equipment_graphics::aircraft};
use replay_data::{rows::TableId, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AircraftScope {
    Aircraft,
    All,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AircraftCatalogueParams {
    pub scope: AircraftScope,
    #[doc = " Matched without case against names, titles and what references them;"]
    #[doc = " empty matches every name."]
    pub query: String,
    #[doc = " The first matching entry to answer."]
    pub first: u32,
    #[doc = " How many matching entries to answer from `first`."]
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
    #[doc = " Why it is an aircraft's; empty for a name only the complete listing has."]
    pub references: Vec<AircraftReference>,
    #[doc = " Why it cannot be flown."]
    pub refusal: Option<AircraftRefusal>,
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
    #[doc = " How many of the scope's entries match the query."]
    pub matching: u32,
    #[doc = " How many names are an aircraft's, and how many the complete listing has."]
    pub aircraft: u32,
    pub all: u32,
    pub first: u32,
    #[doc = " Matching entries from `first`: flyable ones first, then refused ones,"]
    #[doc = " each by name bytes."]
    pub entries: Vec<AircraftEntry>,
    #[doc = " The scope's refused entries, whatever the query, by kind of reason."]
    pub refusals: Vec<AircraftRefusalCount>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AircraftCatalogueReply);

pub struct AircraftCatalogue;

impl Product for AircraftCatalogue {
    const KEY: &'static str = "aircraft-catalogue";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AircraftCatalogueParams;
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
            AircraftScope::decl(),
            AircraftKind::decl(),
            AircraftReference::decl(),
            AircraftRefusalKind::decl(),
            AircraftRefusal::decl(),
            AircraftEntry::decl(),
            AircraftRefusalCount::decl(),
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

pub struct Built {
    entries: Vec<Listed>,
    aircraft: Vec<usize>,
    all: Vec<usize>,
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
        Ok(Built::new(&catalogue, &designs))
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
    pub fn new(catalogue: &Arc<Catalogue>, designs: &[(String, String)]) -> Self {
        match Content::read(catalogue) {
            Ok(content) => Self::from_content(catalogue, &content, designs),
            Err(why) => Self {
                entries: Vec::new(),
                aircraft: Vec::new(),
                all: Vec::new(),
                warnings: vec![format!("The descriptor roster is refused: {why}")],
            },
        }
    }

    pub fn from_content(
        catalogue: &Catalogue,
        content: &Content,
        designs: &[(String, String)],
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
        let entries: Vec<Listed> = listing
            .entries
            .into_iter()
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
                    name: entry.name,
                };
                Listed {
                    search: search_text(&entry),
                    entry,
                }
            })
            .collect();
        let order = |keep: &dyn Fn(&AircraftEntry) -> bool| {
            let (mut flyable, mut refused): (Vec<usize>, Vec<usize>) = (0..entries.len())
                .filter(|&at| keep(&entries[at].entry))
                .partition(|&at| entries[at].entry.refusal.is_none());
            flyable.append(&mut refused);
            flyable
        };
        let aircraft = order(&|entry| !entry.references.is_empty());
        let all = order(&|_| true);
        let mut warnings = content.warnings.clone();
        warnings.extend(listing.diagnostics);
        Self {
            entries,
            aircraft,
            all,
            warnings,
        }
    }

    fn scope(&self, scope: AircraftScope) -> &[usize] {
        match scope {
            AircraftScope::Aircraft => &self.aircraft,
            AircraftScope::All => &self.all,
        }
    }

    pub fn page(&self, params: &AircraftCatalogueParams) -> AircraftCatalogueReply {
        let scope = self.scope(params.scope);
        let query = params.query.trim().to_lowercase();
        let matching: Vec<usize> = scope
            .iter()
            .copied()
            .filter(|&at| query.is_empty() || self.entries[at].search.contains(&query))
            .collect();
        let first = params.first as usize;
        let entries = matching
            .iter()
            .skip(first)
            .take(params.count as usize)
            .map(|&at| self.entries[at].entry.clone())
            .collect();
        let mut refusals = std::collections::BTreeMap::new();
        for refusal in scope
            .iter()
            .filter_map(|&at| self.entries[at].entry.refusal.as_ref())
        {
            *refusals.entry(refusal.kind).or_insert(0_u32) += 1;
        }
        AircraftCatalogueReply {
            matching: count(matching.len()),
            aircraft: count(self.aircraft.len()),
            all: count(self.all.len()),
            first: params.first,
            entries,
            refusals: refusals
                .into_iter()
                .map(|(kind, count)| AircraftRefusalCount {
                    kind,
                    label: refusal_label(kind).to_owned(),
                    count,
                })
                .collect(),
            warnings: self.warnings.clone(),
        }
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
