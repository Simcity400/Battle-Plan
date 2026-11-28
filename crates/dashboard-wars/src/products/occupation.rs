
pub mod detail;
mod law;
pub mod states;

#[cfg(test)]
mod tests;

use dashboard_base::{Source, catalogue::{self, CachedResult, Catalogue}, products::LoadContext};
use content_reader_key_names::equipment::Database;
use content_reader_politics::occupation_laws::Laws;
use replay_data::rows::{CountryLinkRow, StateResistanceRow, TableId};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, sync::{Arc, Mutex, OnceLock}};

pub use law::OccupationLawInfo;

type Named = (Vec<u8>, Vec<u8>, Result<String, String>);

type Judgements = HashMap<(String, u32, u32), (Result<bool, String>, dashboard_text::triggers::inputs::Inputs)>;

#[derive(Default)]
struct Judged {
    at: Option<(replay_data::BranchId, replay_data::Hour)>,
    inputs_at: replay_data::Hour,
    stage: usize,
    through: replay_data::Hour,
    read: WorldReads,
    decisions: Judgements,
    carried: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WorldReads {
    fields: BTreeMap<TableId, u64>,
    ends: BTreeSet<TableId>,
    clock: bool,
    undeclared: bool,
}

impl WorldReads {
    fn note(&mut self, inputs: &dashboard_text::triggers::inputs::Inputs) {
        use dashboard_text::triggers::{inputs::Key, world::Query};
        for key in inputs.keys() {
            if matches!(key, Key::World(Query::Clock)) {
                self.clock = true;
                continue;
            }
            match key.dependence() {
                Some(reads) => for read in reads {
                    *self.fields.entry(read.table).or_default() |= read.fields;
                    if read.ends { self.ends.insert(read.table); }
                },
                None => self.undeclared = true,
            }
        }
    }

    fn add(&mut self, other: &Self) {
        for (&table, &fields) in &other.fields { *self.fields.entry(table).or_default() |= fields; }
        self.ends.extend(&other.ends);
        self.undeclared |= other.undeclared;
    }
}

pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    laws: Result<Arc<Laws>, String>,
    equipment: OnceLock<Result<Arc<Database>, String>>,
    red: Option<i64>,
    warnings: Vec<String>,
    names: Mutex<HashMap<u32, Named>>,
    world: Mutex<WorldReads>,
    judged: Mutex<Judged>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Recorded content stack unavailable; choose matching game and user folders ({why})"));
        let mut warnings = Vec::new();
        let (mut laws, mut red) = (Err("Occupation laws need the recorded content stack".to_owned()), None);
        if let Ok(cat) = &catalogue {
            warnings.extend(cat.verification.iter().cloned());
            warnings.extend(cat.warnings.iter().cloned());
            let layers = &cat.content.layers;
            laws = layers.derived::<CachedResult<Laws>>().get_or_init(|| content_reader_politics::occupation_laws::read(layers)).map_err(|why| why.to_string());
            match &laws {
                Ok(laws) if !laws.warnings.is_empty() => warnings.push(format!("{} occupation-law files could not be read: {}", laws.warnings.len(), laws.warnings.join("; "))),
                Ok(_) => {}
                Err(why) => warnings.push(format!("Occupation law content unavailable: {why}")),
            }
            match cat.defines() {
                Ok(defines) => red = defines.fixed5("NInterface", "GARRISON_STRENGTH_TO_SHOW_RED").map_err(|why| warnings.push(format!("Garrison red threshold unavailable: {why}"))).ok(),
                Err(why) => warnings.push(format!("Garrison defines unavailable: {why}")),
            }
        }
        Self { catalogue, laws, equipment: OnceLock::new(), red, warnings, names: Mutex::default(), world: Mutex::default(), judged: Mutex::default() }
    }

    fn equipment(&self) -> Result<&Database, String> {
        let read = self.equipment.get_or_init(|| {
            let cat = self.catalogue.as_ref().map_err(|_| "Garrison equipment needs the recorded content stack".to_owned())?;
            let layers = &cat.content.layers;
            layers.derived::<CachedResult<Database>>().get_or_init_shared(|| Database::shared_complete(layers)).map_err(|why| why.to_string())
        });
        read.as_deref().map_err(Clone::clone)
    }

    fn catalogue(&self) -> Result<&Catalogue, &str> {
        self.catalogue.as_deref().map_err(String::as_str)
    }
}

pub fn check_laws(cat: &Catalogue) -> Result<Vec<String>, String> {
    let laws = content_reader_politics::occupation_laws::read(&cat.content.layers).map_err(|why| why.to_string())?;
    if laws.warnings.is_empty() {
        for token in laws.definitions.keys() {
            if let Some(why) = law::info(Ok(cat), Ok(&laws), token).missing {
                cat.refuse(&why, &format!("law {token}"));
            }
        }
    }
    Ok(laws.warnings)
}

pub(super) fn occupied(row: &StateResistanceRow) -> bool {
    row.active != 0 && (row.country as i32) > 0
}

fn slot(links: &[CountryLinkRow], raw: u32) -> Option<u32> {
    dashboard_text::metadata::link_slot(links, raw)
}
