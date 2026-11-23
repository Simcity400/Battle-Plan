use super::{Compiled, cache};
use content_reader_layers::content::diagnostics::{Diagnostics as ContentDiagnostics, Observed, Once as OnceLock};
use content_reader_layers::ResolvedLayers;
use content_reader_image::native::Decode;
use content_reader_gui::pdxshader::program::{self, Program};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub effect: Arc<Effect>,
    pub index: usize,
}

impl Selection {
    pub fn fall_back(&self, why: &str) -> Option<Selection> {
        let name = &self.effect.source.techniques[self.index].name;
        self.effect.fall_back(why)?.select(name).ok().flatten()
    }

    pub fn recorded(&self) -> Option<Selection> {
        let name = &self.effect.source.techniques[self.index].name;
        let recorded = &self.effect.recorded.as_ref()?.effect;
        recorded.select(name).ok().flatten()
    }
}

pub fn with_recorded_selection<T, E: std::fmt::Display>(
    selection: &Selection,
    mut build: impl FnMut(&Selection) -> Result<T, E>,
) -> Result<T, E> {
    build(selection).or_else(|error| match selection.fall_back(&error.to_string()) {
        Some(recorded) => build(&recorded),
        None => Err(error),
    })
}

pub struct Effect {
    pub(crate) id: uuid::Uuid,
    pub source: Arc<Program>,
    pub defines: Vec<String>,
    globals: usize,
    compiled: Vec<OnceLock<Result<Arc<Compiled>, String>>>,
    independent: Vec<DrawIndependence>,
    cache: Arc<cache::Cache>,
    variants: Mutex<BTreeMap<Vec<String>, Observed<Arc<Effect>>>>,
    recorded: Option<Recorded>,
}

#[derive(Default)]
struct DrawIndependence {
    ready: std::sync::OnceLock<bool>,
    pending: std::sync::OnceLock<()>,
}

#[derive(Clone)]
pub struct Recorded {
    pub(crate) effect: Arc<Effect>,
    pub(crate) note: Arc<dyn Fn(String) + Send + Sync>,
}

fn refusal(shown: &Effect, base: &Effect) -> Option<String> {
    for (index, technique) in base.source.techniques.iter().enumerate() {
        if base.compiled(index).is_err() {
            continue;
        }
        let Some(own) = shown
            .source
            .techniques
            .iter()
            .position(|candidate| candidate.name.eq_ignore_ascii_case(&technique.name))
        else {
            return Some(format!("its technique {} is missing", technique.name));
        };
        if let Err(why) = shown.compiled(own) {
            return Some(format!(
                "its technique {} does not compile: {why}",
                technique.name
            ));
        }
    }
    None
}

pub fn with_recorded<T>(
    effect: &Arc<Effect>,
    mut build: impl FnMut(&Arc<Effect>) -> std::io::Result<T>,
) -> std::io::Result<T> {
    build(effect).or_else(|error| match effect.fall_back(&error.to_string()) {
        Some(recorded) => build(&recorded),
        None => Err(error),
    })
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
        Self::with_globals(source, Vec::new(), defines, cache, None)
    }
    pub(crate) fn with_globals(
        source: Arc<Program>,
        globals: Vec<String>,
        caller: Vec<String>,
        cache: Arc<cache::Cache>,
        recorded: Option<Recorded>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
            compiled: source.techniques.iter().map(|_| OnceLock::new()).collect(),
            independent: source
                .techniques
                .iter()
                .map(|_| Default::default())
                .collect(),
            source,
            globals: globals.len(),
            defines: globals.into_iter().chain(caller).collect(),
            cache,
            variants: Mutex::new(BTreeMap::new()),
            recorded,
        })
    }

    pub fn fall_back(&self, why: &str) -> Option<Arc<Effect>> {
        let recorded = self.recorded.as_ref()?;
        (recorded.note)(format!("shader {} ({why})", self.source.effect));
        Some(recorded.effect.clone())
    }
    pub fn from_compiled(
        source: Arc<Program>,
        defines: Vec<String>,
        programs: Vec<Result<Arc<Compiled>, String>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
            independent: source
                .techniques
                .iter()
                .map(|_| Default::default())
                .collect(),
            source,
            defines,
            globals: 0,
            compiled: programs.into_iter().map(OnceLock::from).collect(),
            cache: Arc::new(cache::Cache::default()),
            variants: Mutex::new(BTreeMap::new()),
            recorded: None,
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn replacing(
        source: Arc<Program>,
        programs: Vec<Result<Arc<Compiled>, String>>,
        recorded: Arc<Effect>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
            independent: source
                .techniques
                .iter()
                .map(|_| Default::default())
                .collect(),
            source,
            defines: Vec::new(),
            globals: 0,
            compiled: programs.into_iter().map(OnceLock::from).collect(),
            cache: Arc::new(cache::Cache::default()),
            variants: Mutex::new(BTreeMap::new()),
            recorded: Some(Recorded {
                effect: recorded,
                note: Arc::new(|_| {}),
            }),
        })
    }

    pub fn caller(&self) -> &[String] {
        &self.defines[self.globals..]
    }

    pub fn variant(self: &Arc<Self>, caller: Vec<String>) -> Arc<Self> {
        if caller == self.caller() {
            return self.clone();
        }
        self.variants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(caller.clone())
            .or_insert_with(|| {
                Observed::new(|| {
                    let globals = self.defines[..self.globals].to_vec();
                    let Some(recorded) = &self.recorded else {
                        return Self::with_globals(
                            self.source.clone(),
                            globals,
                            caller,
                            self.cache.clone(),
                            None,
                        );
                    };
                    let base = recorded.effect.variant(caller.clone());
                    let shown = Self::with_globals(
                        self.source.clone(),
                        globals,
                        caller,
                        self.cache.clone(),
                        Some(Recorded {
                            effect: base.clone(),
                            note: recorded.note.clone(),
                        }),
                    );
                    match refusal(&shown, &base) {
                        None => shown,
                        Some(why) => {
                            (recorded.note)(format!(
                                "shader {} with {:?} ({why})",
                                self.source.effect,
                                shown.caller()
                            ));
                            base
                        }
                    }
                })
            })
            .get()
            .clone()
    }

    pub fn compiled(&self, index: usize) -> Result<Arc<Compiled>, String> {
        let slot = self
            .compiled
            .get(index)
            .ok_or_else(|| format!("effect {:?}: invalid technique {index}", self.source.effect))?;
        let result = slot
            .get_or_init(|| {
                super::gpu::stats::build();
                self.cache
                    .get(&self.source, &self.source.techniques[index], &self.defines)
            })
            .clone();
        if let Err(reason) = &result {
            self.cache.refuse(reason);
        }
        result
    }
    pub fn compiled_now(&self, index: usize) -> Option<Result<Arc<Compiled>, String>> {
        self.compiled.get(index)?.peek().cloned()
    }

    pub fn independent_draws(&self, index: usize) -> bool {
        self.independence(index).unwrap_or_else(|| {
            if let Some(slot) = self.independent.get(index) {
                slot.pending.get_or_init(|| {
                    eprintln!("native batching effect {:?} technique {:?}: retained draw boundaries while compilation is pending",
                        self.source.effect, self.source.techniques[index].name);
                });
            }
            false
        })
    }

    fn independence(&self, index: usize) -> Option<bool> {
        let slot = self.independent.get(index)?;
        if let Some(independent) = slot.ready.get() {
            return Some(*independent);
        }
        let own = self.compiled_now(index)?;
        let mut independent = own.is_ok_and(|compiled| {
            compiled.vertex.independent_draws && compiled.pixel.independent_draws
        });
        if let Some(recorded) = &self.recorded {
            let name = &self.source.techniques[index].name;
            independent &= match recorded
                .effect
                .source
                .techniques
                .iter()
                .position(|technique| technique.name.eq_ignore_ascii_case(name))
            {
                Some(index) => recorded.effect.independence(index)?,
                None => false,
            };
        }
        Some(*slot.ready.get_or_init(|| {
            eprintln!(
                "native batching effect {:?} technique {:?}: {}",
                self.source.effect,
                self.source.techniques[index].name,
                if independent {
                    "independent draws"
                } else {
                    "retained draw boundaries"
                }
            );
            independent
        }))
    }

    pub fn warm(&self) {
        for (index, slot) in self.compiled.iter().enumerate() {
            slot.get_or_init(|| {
                super::gpu::stats::build();
                self.cache
                    .get(&self.source, &self.source.techniques[index], &self.defines)
            });
        }
    }

    pub(crate) fn failed(&self) -> bool {
        self.compiled
            .iter()
            .any(|slot| matches!(slot.peek(), Some(Err(_))))
    }
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

pub fn globals(size: [u32; 2], decode: Decode) -> Vec<String> {
    let value = |name: &str, value: f32| format!("{name} {value:.3}");
    let [width, height] = size.map(|side| side as f32);
    let mut defines = vec![
        value("MAP_SIZE_X", width),
        value("MAP_SIZE_Y", height),
        "GRADIENT_BORDERS".into(),
    ];
    if !decode.force_pow2_textures {
        defines.extend([
            value("MAP_POW2_X", 1.0),
            value("MAP_POW2_Y", 1.0),
            value("FOW_POW2_X", 1.0),
            value("FOW_POW2_Y", 1.0),
        ]);
        return defines;
    }
    let scale = |side: f32| side / next_power_of_two(side);
    let fog = [width, height].map(|side| side / 6.0);
    defines.extend([
        value("FOW_POW2_X", scale(fog[0])),
        value("FOW_POW2_Y", scale(fog[1])),
    ]);
    let limit = RENDERER_TEXTURE_LIMIT as f32;
    let map = if width > limit || height > limit {
        [width * 0.5, height * 0.5]
    } else {
        [width, height]
    };
    defines.extend([
        value("MAP_POW2_X", scale(map[0])),
        value("MAP_POW2_Y", scale(map[1])),
    ]);
    defines
}

const RENDERER_TEXTURE_LIMIT: u32 = 0x4000;

fn next_power_of_two(side: f32) -> f32 {
    let truncated = if side.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&side) {
        i32::MIN
    } else {
        side as i32
    };
    let mut bits = truncated.wrapping_sub(1) as u32;
    for shift in [1, 2, 4, 8, 16] {
        bits |= bits >> shift;
    }
    bits.wrapping_add(1) as f32
}

type Variant = (Vec<u8>, Vec<String>);

pub struct Library {
    layers: Arc<ResolvedLayers>,
    globals: Vec<String>,
    pub cache: Arc<cache::Cache>,
    sources: BTreeMap<Vec<u8>, Observed<Result<Arc<Program>, String>>>,
    effects: BTreeMap<Variant, Observed<Result<Arc<Effect>, String>>>,
    pub refused: BTreeMap<String, u64>,
    pub warnings: BTreeSet<String>,
    recorded: Option<Option<Arc<ResolvedLayers>>>,
    recorded_sources: BTreeMap<Vec<u8>, Observed<Result<Arc<Program>, String>>>,
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
            recorded: None,
            recorded_sources: BTreeMap::new(),
        }
    }
    pub fn with_globals(mut self, globals: Vec<String>) -> Self {
        self.globals = globals;
        self
    }

    pub fn load(&mut self, file: &[u8], defines: &[String]) -> Result<Arc<Effect>, String> {
        let key = (file.to_owned(), defines.to_vec());
        if let Some(effect) = self.effects.get(&key) {
            let result = effect.get().clone();
            if let Err(reason) = &result {
                *self.refused.entry(reason.clone()).or_default() += 1;
            }
            return result;
        }
        let entry = Observed::new(|| self.load_uncached(file, defines));
        let result = entry.get().clone();
        self.effects.insert(key, entry);
        result
    }

    fn load_uncached(&mut self, file: &[u8], defines: &[String]) -> Result<Arc<Effect>, String> {
        let shown = self
            .sources
            .entry(file.to_owned())
            .or_insert_with(|| Observed::new(|| program::read(&self.layers, file).map(Arc::new)))
            .get()
            .clone();
        let recorded = self
            .recorded
            .get_or_insert_with(|| self.layers.without_overlays().map(Arc::new))
            .clone();
        let effect = |source: Arc<Program>, recorded: Option<Recorded>| {
            Effect::with_globals(
                source,
                self.globals.clone(),
                defines.to_vec(),
                self.cache.clone(),
                recorded,
            )
        };
        let layers = self.layers.clone();
        let note: Arc<dyn Fn(String) + Send + Sync> =
            Arc::new(move |refusal| layers.note_overlay_refusal(refusal));
        let refused = |why: &dyn std::fmt::Display| {
            note(format!("shader {} ({why})", String::from_utf8_lossy(file)));
        };
        let result = match recorded {
            None => shown.map(|shown| effect(shown, None)),
            Some(recorded) => {
                let base = self
                    .recorded_sources
                    .entry(file.to_owned())
                    .or_insert_with(|| {
                        Observed::new(|| program::read(&recorded, file).map(Arc::new))
                    })
                    .get()
                    .clone();
                match (shown, base) {
                    (Ok(shown), Ok(base)) if shown.identity == base.identity => {
                        Ok(effect(shown, None))
                    }
                    (Ok(shown), Ok(base)) => {
                        let base = effect(base, None);
                        let shown = effect(
                            shown,
                            Some(Recorded {
                                effect: base.clone(),
                                note: note.clone(),
                            }),
                        );
                        match refusal(&shown, &base) {
                            None => Ok(shown),
                            Some(why) => {
                                refused(&why);
                                Ok(base)
                            }
                        }
                    }
                    (Err(why), Ok(base)) => {
                        refused(&why);
                        Ok(effect(base, None))
                    }
                    (shown, Err(_)) => shown.map(|shown| effect(shown, None)),
                }
            }
        };
        if let Ok(effect) = &result {
            for warning in &effect.source.warnings {
                ContentDiagnostics::shader(warning.clone());
                if self.warnings.insert(warning.clone()) {
                    eprintln!("{warning}");
                }
            }
        }
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_variants_and_pipelines_fall_back_to_the_recorded_effect() {
        struct Picky(cache::tests::Count, [u8; 32]);
        impl cache::Compile for Picky {
            fn compile(
                &self,
                source: &Program,
                technique: &content_reader_gui::pdxshader::program::Technique,
                defines: &[String],
            ) -> Result<super::super::Compiled, String> {
                if source.identity == self.1 && defines.iter().any(|define| define == "ANIMATED") {
                    return Err("the replacement does not compile when animated".into());
                }
                self.0.compile(source, technique, defines)
            }
        }
        let recorded = Arc::new(cache::tests::source("static const float4 C = 1;"));
        let replacement = Arc::new(cache::tests::source("static const float4 C = 2;"));
        assert_ne!(recorded.identity, replacement.identity);
        let cache = Arc::new(cache::Cache::new(
            None,
            Arc::new(Picky(Default::default(), replacement.identity)),
        ));
        let notes = Arc::new(Mutex::new(Vec::new()));
        let sink = notes.clone();
        let base = Effect::with_globals(recorded, vec![], vec![], cache.clone(), None);
        let shown = Effect::with_globals(
            replacement,
            vec![],
            vec![],
            cache,
            Some(Recorded {
                effect: base.clone(),
                note: Arc::new(move |note| sink.lock().unwrap().push(note)),
            }),
        );
        assert!(refusal(&shown, &base).is_none());
        let animated = shown.variant(vec!["ANIMATED".into()]);
        assert!(Arc::ptr_eq(
            &animated,
            &base.variant(vec!["ANIMATED".into()])
        ));
        assert_eq!(notes.lock().unwrap().len(), 1);
        assert!(notes.lock().unwrap()[0].contains("does not compile when animated"));
        let built = with_recorded(&shown, |effect| {
            if Arc::ptr_eq(effect, &shown) {
                Err(std::io::Error::other(
                    "native streams cannot supply its vertex input",
                ))
            } else {
                Ok(effect.clone())
            }
        })
        .unwrap();
        assert!(Arc::ptr_eq(&built, &base));
        assert!(notes.lock().unwrap()[1].contains("vertex input"));
        let selection = shown.select("Up").unwrap().unwrap();
        let built = with_recorded_selection(&selection, |selection| {
            if Arc::ptr_eq(&selection.effect, &shown) {
                Err("its blend state cannot be built")
            } else {
                Ok((selection.effect.clone(), selection.index))
            }
        })
        .unwrap();
        assert!(Arc::ptr_eq(&built.0, &base));
        assert_eq!(base.source.techniques[built.1].name, "Up");
        assert!(with_recorded(&base, |_| Err::<(), _>(std::io::Error::other("no"))).is_err());
    }

    #[test]
    fn variants_keep_the_globals() {
        let source = Arc::new(program::read_with::<Vec<u8>>(b"gfx/FX/v.lua", &|_| None).unwrap());
        let cache = Arc::new(cache::Cache::new(
            None,
            Arc::new(cache::tests::Count::default()),
        ));
        let globals = globals([4, 2], Decode::default());
        let base =
            Effect::with_globals(source.clone(), globals.clone(), vec![], cache.clone(), None);
        let loaded = Effect::with_globals(
            source,
            globals.clone(),
            vec!["ANIMATED".into()],
            cache,
            None,
        );
        let derived = base.variant(vec!["ANIMATED".into()]);
        assert_eq!(derived.defines, loaded.defines);
        assert_eq!(derived.caller(), ["ANIMATED"]);
        assert_eq!(derived.defines[..globals.len()], globals[..]);
        assert_eq!(derived.variant(vec![]).defines, globals);
        assert!(Arc::ptr_eq(&base, &base.variant(vec![])));
    }

    #[test]
    fn globals_follow_the_native_order_and_format() {
        let force = Decode {
            force_pow2_textures: true,
        };
        assert_eq!(
            super::globals([5632, 2048], Decode::default()),
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
        assert_eq!(
            super::globals([5632, 2048], force),
            [
                "MAP_SIZE_X 5632.000",
                "MAP_SIZE_Y 2048.000",
                "GRADIENT_BORDERS",
                "FOW_POW2_X 0.917",
                "FOW_POW2_Y 0.667",
                "MAP_POW2_X 0.688",
                "MAP_POW2_Y 1.000",
            ]
        );
        assert_eq!(
            super::globals([16384, 16385], force)[3..],
            [
                "FOW_POW2_X 0.667",
                "FOW_POW2_Y 0.667",
                "MAP_POW2_X 1.000",
                "MAP_POW2_Y 1.000",
            ]
        );
        assert_eq!(super::next_power_of_two(0.5), 0.0);
        assert_eq!(super::next_power_of_two(1.0), 1.0);
        assert_eq!(super::next_power_of_two(1024.9), 1024.0);
        assert_eq!(super::next_power_of_two(1025.0), 2048.0);
    }
}
