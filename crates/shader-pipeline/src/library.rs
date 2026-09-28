//! Content-scoped lazy effect assets. Parsing happens on load; only selected techniques compile.
use super::{Compiled, cache};
use installed_content::{
    ResolvedLayers,
    pdxshader::program::{self, Program},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub effect: Arc<Effect>,
    pub index: usize,
}

pub struct Effect {
    pub(crate) id: uuid::Uuid,
    pub source: Arc<Program>,
    /// Every define the effect compiles with: the library's globals, then the caller's.
    pub defines: Vec<String>,
    /// How many of `defines` are the library's globals, which every variant keeps.
    globals: usize,
    compiled: Vec<OnceLock<Result<Arc<Compiled>, String>>>,
    cache: Arc<cache::Cache>,
    variants: Mutex<BTreeMap<Vec<String>, Arc<Effect>>>,
}
impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effect")
            .field("source", &self.source.effect)
            .field("defines", &self.defines)
            .finish()
    }
}
impl PartialEq for Effect {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Effect {
    pub fn new(source: Arc<Program>, defines: Vec<String>, cache: Arc<cache::Cache>) -> Arc<Self> {
        Self::with_globals(source, Vec::new(), defines, cache)
    }
    fn with_globals(
        source: Arc<Program>,
        globals: Vec<String>,
        caller: Vec<String>,
        cache: Arc<cache::Cache>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
            compiled: source.techniques.iter().map(|_| OnceLock::new()).collect(),
            source,
            globals: globals.len(),
            defines: globals.into_iter().chain(caller).collect(),
            cache,
            variants: Mutex::new(BTreeMap::new()),
        })
    }
    /// Offline oracle/precompiled caller path; normal runtime uses `Library::load`.
    pub fn from_compiled(
        source: Arc<Program>,
        defines: Vec<String>,
        programs: Vec<Result<Arc<Compiled>, String>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
            source,
            defines,
            globals: 0,
            compiled: programs.into_iter().map(OnceLock::from).collect(),
            cache: Arc::new(cache::Cache::default()),
            variants: Mutex::new(BTreeMap::new()),
        })
    }
    /// The caller's defines, after the library's globals.
    pub fn caller(&self) -> &[String] {
        &self.defines[self.globals..]
    }

    /// Same resolved source, globals and persistent cache, with another caller define set.
    /// Variant handles remain stable so device pipelines are reused across frames.
    pub fn variant(self: &Arc<Self>, caller: Vec<String>) -> Arc<Self> {
        if caller == self.caller() {
            return self.clone();
        }
        self.variants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(caller.clone())
            .or_insert_with(|| {
                let globals = self.defines[..self.globals].to_vec();
                Self::with_globals(self.source.clone(), globals, caller, self.cache.clone())
            })
            .clone()
    }

    pub fn compiled(&self, index: usize) -> Result<Arc<Compiled>, String> {
        let slot = self
            .compiled
            .get(index)
            .ok_or_else(|| format!("effect {:?}: invalid technique {index}", self.source.effect))?;
        let result = slot
            .get_or_init(|| {
                self.cache
                    .get(&self.source, &self.source.techniques[index], &self.defines)
            })
            .clone();
        if let Err(reason) = &result {
            self.cache.refuse(reason);
        }
        result
    }
    /// Missing technique retains native state; a declared technique that fails is refused.
    pub fn select(self: &Arc<Self>, name: &str) -> Result<Option<Selection>, String> {
        let Some(index) = self
            .source
            .techniques
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
        else {
            return Ok(None);
        };
        self.compiled(index)?;
        Ok(Some(Selection {
            effect: self.clone(),
            index,
        }))
    }
}

/// The renderer's global defines, which `engine_shader_effect_lookup@1423F4EC0` puts before
/// every effect's caller defines, GUI and map alike (shader cache `+0x32030`), as
/// `derive_map_shader_constants@140A4DDE0` writes them for a map of `size` (the provinces bitmap,
/// `CMap+0x40`/`+0x44`), each value `%.3f` (`1424B5850`, format `142972FBC`). The power-of-two
/// scales take the branch for `force_pow2_textures = no`, the settings default: the recording
/// holds no user settings, and the other branch also needs the device's texture limit.
pub fn globals(size: [u32; 2]) -> Vec<String> {
    let value = |name: &str, value: f32| format!("{name} {value:.3}");
    vec![
        value("MAP_SIZE_X", size[0] as f32),
        value("MAP_SIZE_Y", size[1] as f32),
        "GRADIENT_BORDERS".into(),
        value("MAP_POW2_X", 1.0),
        value("MAP_POW2_Y", 1.0),
        value("FOW_POW2_X", 1.0),
        value("FOW_POW2_Y", 1.0),
    ]
}

pub struct Library {
    layers: Arc<ResolvedLayers>,
    globals: Vec<String>,
    pub cache: Arc<cache::Cache>,
    sources: BTreeMap<String, Result<Arc<Program>, String>>,
    effects: BTreeMap<(String, Vec<String>), Result<Arc<Effect>, String>>,
    pub refused: BTreeMap<String, u64>,
    pub warnings: BTreeSet<String>,
}
impl Library {
    pub fn new(layers: Arc<ResolvedLayers>) -> Self {
        Self::with_cache(layers, Arc::new(cache::Cache::default()))
    }
    pub fn with_cache(layers: Arc<ResolvedLayers>, cache: Arc<cache::Cache>) -> Self {
        Self {
            layers,
            globals: Vec::new(),
            cache,
            sources: BTreeMap::new(),
            effects: BTreeMap::new(),
            refused: BTreeMap::new(),
            warnings: BTreeSet::new(),
        }
    }
    /// Every effect this library loads gets `globals` (see [`globals`]) before its caller
    /// defines.
    pub fn with_globals(mut self, globals: Vec<String>) -> Self {
        self.globals = globals;
        self
    }

    pub fn load(&mut self, file: &str, defines: &[String]) -> Result<Arc<Effect>, String> {
        let key = (file.to_owned(), defines.to_vec());
        let result = self
            .effects
            .entry(key)
            .or_insert_with(|| {
                let source = self
                    .sources
                    .entry(file.to_owned())
                    .or_insert_with(|| program::read(&self.layers, file).map(Arc::new))
                    .clone()?;
                for warning in &source.warnings {
                    if self.warnings.insert(warning.clone()) {
                        eprintln!("{warning}");
                    }
                }
                Ok(Effect::with_globals(
                    source,
                    self.globals.clone(),
                    defines.to_vec(),
                    self.cache.clone(),
                ))
            })
            .clone();
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A variant keeps the library's globals: deriving a caller set equals loading it.
    #[test]
    fn variants_keep_the_globals() {
        let source = Arc::new(program::read_with("gfx/FX/v.lua", &|_| None).unwrap());
        let cache = Arc::new(cache::Cache::new(
            None,
            Arc::new(cache::tests::Count::default()),
        ));
        let globals = globals([4, 2]);
        let base = Effect::with_globals(source.clone(), globals.clone(), vec![], cache.clone());
        let loaded = Effect::with_globals(source, globals.clone(), vec!["ANIMATED".into()], cache);
        let derived = base.variant(vec!["ANIMATED".into()]);
        assert_eq!(derived.defines, loaded.defines);
        assert_eq!(derived.caller(), ["ANIMATED"]);
        assert_eq!(derived.defines[..globals.len()], globals[..]);
        // An empty caller set from a variant still keeps the globals.
        assert_eq!(derived.variant(vec![]).defines, globals);
        assert!(Arc::ptr_eq(&base, &base.variant(vec![])));
    }

    #[test]
    fn globals_follow_the_native_order_and_format() {
        assert_eq!(
            super::globals([5632, 2048]),
            [
                "MAP_SIZE_X 5632.000",
                "MAP_SIZE_Y 2048.000",
                "GRADIENT_BORDERS",
                "MAP_POW2_X 1.000",
                "MAP_POW2_Y 1.000",
                "FOW_POW2_X 1.000",
                "FOW_POW2_Y 1.000",
            ]
        );
    }
}
