
use content_reader_layers::content::{ResolveOptions, facts::{Pack, PackSource}, playback};
use content_reader_defines::defines;
use content_reader_key_names::unit_leader::needs::SubUnitNeeds;
use content_reader_province_area::province_area::ProvinceAreas;
use content_reader_regions::regions as strategic_regions;
use replay_data::identity::Identity;
use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Definition {
    ProvinceAreas,
    Define(Define),
    SubUnitNeeds,
    StrategicRegions,
}

impl fmt::Display for Definition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProvinceAreas => formatter.write_str("province areas"),
            Self::Define(define) => write!(formatter, "define {}.{}", define.section, define.name),
            Self::SubUnitNeeds => formatter.write_str("sub-unit equipment needs"),
            Self::StrategicRegions => formatter.write_str("strategic regions"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Define {
    pub section: &'static str,
    pub name: &'static str,
    pub form: Form,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Int32,
    Fixed5,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceRegions {
    regions: Vec<(u32, u32)>,
}

impl ProvinceRegions {
    pub fn new<'a>(regions: impl IntoIterator<Item = (u32, &'a [u32])>) -> Self {
        let mut claimed: std::collections::BTreeMap<u32, Option<u32>> = std::collections::BTreeMap::new();
        for (region, provinces) in regions {
            for &province in provinces {
                claimed.entry(province).and_modify(|held| if *held != Some(region) { *held = None }).or_insert(Some(region));
            }
        }
        Self { regions: claimed.into_iter().filter_map(|(province, region)| Some((province, region?))).collect() }
    }

    pub fn of(&self, province: u32) -> Option<u32> {
        self.regions.binary_search_by_key(&province, |&(held, _)| held).ok().map(|at| self.regions[at].1)
    }

    fn bytes(&self) -> usize {
        self.regions.capacity() * 8
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Uses {
    pub definition: Definition,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definitions {
    areas: Result<Vec<i64>, String>,
    defines: Vec<(Define, Result<i64, String>)>,
    defines_absent: String,
    needs: Result<SubUnitNeeds, String>,
    regions: Result<ProvinceRegions, String>,
}

const UNDECLARED: &str = "no registered writer declares it";

impl Definitions {
    pub fn absent(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            areas: Err(reason.clone()),
            defines: Vec::new(),
            defines_absent: reason.clone(),
            needs: Err(reason.clone()),
            regions: Err(reason),
        }
    }

    pub fn unpacked() -> Self {
        Self::absent("the session's game facts are not packed yet")
    }

    pub fn read_none() -> Self {
        Self::absent(UNDECLARED)
    }

    pub fn with_areas(mut self, areas: Result<Vec<i64>, String>) -> Self {
        self.areas = areas;
        self
    }

    pub fn with_define(mut self, define: Define, value: Result<i64, String>) -> Self {
        self.defines.retain(|(held, _)| (held.section, held.name) != (define.section, define.name));
        self.defines.push((define, value));
        self
    }

    pub fn with_needs(mut self, needs: Result<SubUnitNeeds, String>) -> Self {
        self.needs = needs;
        self
    }

    pub fn with_regions(mut self, regions: Result<ProvinceRegions, String>) -> Self {
        self.regions = regions;
        self
    }

    pub fn areas(&self) -> Result<&[i64], &str> {
        self.areas.as_deref().map_err(String::as_str)
    }

    pub fn define(&self, section: &str, name: &str) -> Result<i64, &str> {
        match self.defines.iter().find(|(define, _)| define.section == section && define.name == name) {
            Some((_, value)) => value.as_ref().copied().map_err(String::as_str),
            None => Err(&self.defines_absent),
        }
    }

    pub fn needs(&self) -> Result<&SubUnitNeeds, &str> {
        self.needs.as_ref().map_err(String::as_str)
    }

    pub fn regions(&self) -> Result<&ProvinceRegions, &str> {
        self.regions.as_ref().map_err(String::as_str)
    }

    pub fn reason(&self, definition: &Definition) -> Option<&str> {
        match definition {
            Definition::ProvinceAreas => self.areas().err(),
            Definition::Define(define) => self.define(define.section, define.name).err(),
            Definition::SubUnitNeeds => self.needs().err(),
            Definition::StrategicRegions => self.regions().err(),
        }
    }

    pub fn bytes(&self) -> usize {
        let areas = self.areas.as_ref().map_or(0, |areas| areas.capacity() * 8);
        let defines = self.defines.len() * 64;
        let needs = self.needs.as_ref().map_or(0, |needs| {
            needs.needs.iter().map(|(key, need)| 72 + key.len() + need.iter().map(|(archetype, _)| 32 + archetype.len()).sum::<usize>()).sum::<usize>()
                + needs.dropped.iter().map(String::len).sum::<usize>()
        });
        areas + defines + needs + self.regions.as_ref().map_or(0, ProvinceRegions::bytes)
    }
}

pub struct Facts {
    pub source: Arc<dyn PackSource>,
    pub length: u64,
}

impl Facts {
    pub fn held(bytes: Arc<Vec<u8>>) -> Self {
        let length = bytes.len() as u64;
        Self { source: bytes, length }
    }

    pub fn stored(file: File, offset: u64, length: u64) -> Self {
        Self { source: Arc::new(Stored { file: Mutex::new(file), offset }), length }
    }

    pub fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.length) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "a read past the game facts pack"));
        }
        self.source.read_at(offset, out)
    }
}

struct Stored {
    file: Mutex<File>,
    offset: u64,
}

impl PackSource for Stored {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        file.seek(SeekFrom::Start(self.offset + offset))?;
        file.read_exact(out)
    }
}

pub type Reader = fn(&Facts, &Identity, &[Definition]) -> Definitions;

pub fn read(facts: &Facts, identity: &Identity, wanted: &[Definition]) -> Definitions {
    let opened = match Pack::open(Arc::clone(&facts.source), 0, facts.length) {
        Ok(opened) => Arc::new(opened.keeping(0)),
        Err(error) => return Definitions::absent(format!("the recording's game facts cannot be read: {error}")),
    };
    if let Some(why) = opened.binding_differs(identity) {
        return Definitions::absent(why);
    }
    let tables = opened.tables();
    let options = ResolveOptions { game_dir: PathBuf::new(), user_dir: PathBuf::new(), cache_dir: PathBuf::new() };
    let content = match playback(identity, Ok(Arc::clone(&opened)), &options) {
        Ok(content) => content,
        Err(error) => return Definitions::absent(format!("the recording's game facts cannot be mounted: {error}")),
    };
    let layers = &content.layers;
    engine_executable::executable_tables::scope(tables.clone().ok(), || {
        let mut out = Definitions::read_none();
        if wanted.contains(&Definition::ProvinceAreas) {
            out.areas = guarded(|| ProvinceAreas::read(layers).map(|areas| areas.land).map_err(|error| format!("the province areas cannot be read from the recording's game facts: {error}")));
        }
        let asked: Vec<Define> = wanted
            .iter()
            .filter_map(|definition| match definition {
                Definition::Define(define) => Some(*define),
                _ => None,
            })
            .collect();
        if !asked.is_empty() {
            match guarded(|| defines::read(layers).map_err(|error| format!("the defines cannot be read from the recording's game facts: {error}"))) {
                Ok(read) => {
                    for define in asked {
                        let value = match define.form {
                            Form::Int32 => read.int32(define.section, define.name).map(i64::from),
                            Form::Fixed5 => read.fixed5(define.section, define.name),
                        };
                        out = out.with_define(define, value.map_err(|error| format!("{}.{} in the recording's game facts: {error}", define.section, define.name)));
                    }
                }
                Err(why) => out.defines_absent = why,
            }
        }
        if wanted.contains(&Definition::SubUnitNeeds) {
            out.needs = guarded(|| content_reader_key_names::equipment_facts::sub_unit_needs(&opened));
        }
        if wanted.contains(&Definition::StrategicRegions) {
            out.regions = guarded(|| {
                strategic_regions::read(layers)
                    .map(|read| ProvinceRegions::new(read.regions.values().map(|region| (region.id, region.provinces.as_slice()))))
                    .map_err(|error| format!("the strategic regions cannot be read from the recording's game facts: {error}"))
            });
        }
        out
    })
}

fn guarded<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|panic| {
        let text = panic.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
        Err(format!("its reader stopped: {text}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: replay_data::identity::Verification::Unchecked,
        }
    }

    #[test]
    fn an_unreadable_pack_withholds_every_kind_by_name() {
        let read = read(&Facts::held(Arc::new(b"not a pack".to_vec())), &identity(), &[Definition::ProvinceAreas, Definition::SubUnitNeeds, Definition::StrategicRegions]);
        let reason = read.areas().unwrap_err();
        assert!(reason.contains("game facts cannot be read"), "{reason}");
        assert_eq!(read.define("NMilitary", "LOW_SUPPLY"), Err(reason));
        assert_eq!(read.needs().unwrap_err(), reason);
        assert_eq!(read.regions().unwrap_err(), reason);
    }

    #[test]
    fn a_define_not_read_is_absent_by_name_and_one_read_is_held() {
        let low = Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 };
        let held = Definitions::read_none().with_define(low, Ok(25_000));
        assert_eq!(held.define("NMilitary", "LOW_SUPPLY"), Ok(25_000));
        assert_eq!(held.define("NMilitary", "SUPPLY_GRACE"), Err(UNDECLARED));
        assert_eq!(held.reason(&Definition::Define(low)), None);
        assert_eq!(held.reason(&Definition::ProvinceAreas), Some(UNDECLARED));
        assert_eq!(held.reason(&Definition::StrategicRegions), Some(UNDECLARED));
    }

    #[test]
    fn a_province_is_the_one_region_listing_it_and_none_where_several_do() {
        let regions = ProvinceRegions::new([(7, &[100, 101, 101][..]), (8, &[102, 101][..]), (9, &[][..])]);
        assert_eq!((regions.of(100), regions.of(101), regions.of(102), regions.of(103)), (Some(7), None, Some(8), None));
    }
}
