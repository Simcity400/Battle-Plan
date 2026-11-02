//! Content-scoped lazy effect assets. Parsing happens on load; only selected techniques compile.
use super::{Compiled, cache};
use installed_content_core::content::diagnostics::{
    Diagnostics as ContentDiagnostics, Observed, Once as OnceLock,
};
use installed_content_core::{
    ResolvedLayers,
    image::native::Decode,
    pdxshader::program::{self, Program},
};
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
    /// The same technique of the recorded effect a visual mod's replacement stands in for, the
    /// replacement refused for `why`; `None` for an effect no visual mod replaces.
    pub fn fall_back(&self, why: &str) -> Option<Selection> {
        let name = &self.effect.source.techniques[self.index].name;
        self.effect.fall_back(why)?.select(name).ok().flatten()
    }
}

/// [`with_recorded`] for a pass built from a selected technique.
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
    /// Every define the effect compiles with: the library's globals, then the caller's.
    pub defines: Vec<String>,
    /// How many of `defines` are the library's globals, which every variant keeps.
    globals: usize,
    compiled: Vec<OnceLock<Result<Arc<Compiled>, String>>>,
    cache: Arc<cache::Cache>,
    variants: Mutex<BTreeMap<Vec<String>, Observed<Arc<Effect>>>>,
    /// The recorded effect a visual mod's replacement stands in for, with the same defines.
    recorded: Option<Recorded>,
}

/// A replacement effect's recorded counterpart and where a refusal of the replacement is named.
#[derive(Clone)]
pub struct Recorded {
    pub(crate) effect: Arc<Effect>,
    pub(crate) note: Arc<dyn Fn(String) + Send + Sync>,
}

/// Why a mod's replacement cannot stand in for the recorded effect under the same defines: a
/// technique the recorded effect compiles that the replacement lacks or cannot compile.
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

/// `build` with `effect`, or, when that fails and `effect` replaces a recorded effect, with the
/// recorded effect, the replacement refused by name. Passes whose pipelines the content shapes
/// (vertex inputs, states) use it, so a replacement that compiles but cannot be drawn falls back.
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
            source,
            globals: globals.len(),
            defines: globals.into_iter().chain(caller).collect(),
            cache,
            variants: Mutex::new(BTreeMap::new()),
            recorded,
        })
    }

    /// The recorded effect this replacement stands in for, with the replacement refused by name
    /// for `why`; `None` for an effect no visual mod replaces.
    pub fn fall_back(&self, why: &str) -> Option<Arc<Effect>> {
        let recorded = self.recorded.as_ref()?;
        (recorded.note)(format!("shader {} ({why})", self.source.effect));
        Some(recorded.effect.clone())
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
            recorded: None,
        })
    }
    /// A visual mod's replacement standing in for `recorded`, as [`Library::load`] makes one,
    /// from programs already compiled: for other crates' tests of the fallbacks.
    #[cfg(any(test, feature = "test-support"))]
    pub fn replacing(
        source: Arc<Program>,
        programs: Vec<Result<Arc<Compiled>, String>>,
        recorded: Arc<Effect>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4(),
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

    /// The caller's defines, after the library's globals.
    pub fn caller(&self) -> &[String] {
        &self.defines[self.globals..]
    }

    /// Same resolved source, globals and persistent cache, with another caller define set.
    /// Variant handles remain stable so device pipelines are reused across frames. A
    /// replacement's variant is held to the same rule as the replacement itself
    /// ([`Library::load`]): when the recorded effect's variant compiles a technique the
    /// replacement's does not, the recorded variant is used and the replacement refused by name.
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
                self.cache
                    .get(&self.source, &self.source.techniques[index], &self.defines)
            })
            .clone();
        if let Err(reason) = &result {
            self.cache.refuse(reason);
        }
        result
    }
    /// Whether a technique of this effect failed to compile; it keeps that failure.
    pub(crate) fn failed(&self) -> bool {
        self.compiled
            .iter()
            .any(|slot| matches!(slot.peek(), Some(Err(_))))
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

/// The renderer's global defines precede every effect's caller defines, GUI and map alike.
/// Map `size` comes from the provinces bitmap; values use three decimal places, rounded to
/// nearest with ties to even, as Rust's `{:.3}`.
///
/// Power-of-two scales use the client's `force_pow2_textures` (`decode`, the same [`Decode`]
/// its textures load under, `settings::SettingsKeys::texture_decode`): off, all
/// four are `1.0`, `MAP_POW2_X`, `MAP_POW2_Y`, `FOW_POW2_X`, `FOW_POW2_Y` in that order; on,
/// `FOW_POW2_X`/`_Y` first, each fog side (the map's over `6.0`) over the next power of two of
/// its truncation, then `MAP_POW2_X`/`_Y`, each map side over the next power of two of its
/// truncation, both sides halved first when either exceeds `RENDERER_TEXTURE_LIMIT`.
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

/// The renderer's fixed texture-size limit; it is not queried from the device.
const RENDERER_TEXTURE_LIMIT: u32 = 0x4000;

/// Next power of two of `side`, as a float: truncate to a signed 32-bit integer (`i32::MIN`
/// for an unrepresentable value), subtract one, smear bits right by 1, 2, 4, 8 and 16, then add
/// one with 32-bit wrapping (0 wraps to 0) and convert the unsigned result to float.
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

/// An effect as a sprite names it (its name's bytes) with the caller's defines.
type Variant = (Vec<u8>, Vec<String>);

pub struct Library {
    layers: Arc<ResolvedLayers>,
    globals: Vec<String>,
    pub cache: Arc<cache::Cache>,
    /// Each effect's source, by the name's bytes.
    sources: BTreeMap<Vec<u8>, Observed<Result<Arc<Program>, String>>>,
    effects: BTreeMap<Variant, Observed<Result<Arc<Effect>, String>>>,
    pub refused: BTreeMap<String, u64>,
    pub warnings: BTreeSet<String>,
    /// The recorded stack below a presentation stack's visual mods, read once when an effect
    /// is first loaded: `None` inside when no visual mod is mounted.
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
    /// Every effect this library loads gets `globals` (see [`globals`]) before its caller
    /// defines.
    pub fn with_globals(mut self, globals: Vec<String>) -> Self {
        self.globals = globals;
        self
    }

    /// The effect a sprite names, `file` being the name's bytes (opened by them,
    /// [`program::read`]).
    ///
    /// Over a presentation stack, an effect a visual mod replaces is used when every technique
    /// the recorded effect compiles also compiles from the mod's sources; otherwise the
    /// recorded effect is used and the reason is named with the overlay refusals. Effects the
    /// mods leave alone resolve to the same sources and are not compiled twice.
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
                        // The replacement keeps its recorded counterpart, so its variants and
                        // the passes that build pipelines from it can still fall back.
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

    /// A replacement accepted under its loaded defines is held to the same rule for each
    /// variant (here ANIMATED, which it cannot compile): the recorded variant is drawn and the
    /// replacement named. A pass that cannot build a pipeline from an accepted replacement
    /// builds it from the recorded effect.
    #[test]
    fn replacement_variants_and_pipelines_fall_back_to_the_recorded_effect() {
        struct Picky(cache::tests::Count, [u8; 32]);
        impl cache::Compile for Picky {
            fn compile(
                &self,
                source: &Program,
                technique: &installed_content_core::pdxshader::program::Technique,
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
        // A pass built from a selected technique falls back to the same technique.
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
        // An effect no visual mod replaces has nothing to fall back to.
        assert!(with_recorded(&base, |_| Err::<(), _>(std::io::Error::other("no"))).is_err());
    }

    /// A variant keeps the library's globals: deriving a caller set equals loading it.
    #[test]
    fn variants_keep_the_globals() {
        let source = Arc::new(program::read_with(b"gfx/FX/v.lua", &|_| None).unwrap());
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
        // An empty caller set from a variant still keeps the globals.
        assert_eq!(derived.variant(vec![]).defines, globals);
        assert!(Arc::ptr_eq(&base, &base.variant(vec![])));
    }

    /// Both `force_pow2_textures` branches: off, every scale 1 in the map-first
    /// order; on, the fog first, each side over the next power of two of its truncation
    /// (5632 / 6 = 938.667 over 1024; 5632 over 8192 is the tie 0.6875, which ties to even), and
    /// both map sides halved when either exceeds the renderer's 16384 (16385 alone halves
    /// 16384 too: 8192 / 8192, where unhalved it would be 16384 / 16384 and 16385 / 32768).
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
        // The 32-bit wrap: a fog side under 1 truncates to 0, whose next power of two is 0.
        assert_eq!(super::next_power_of_two(0.5), 0.0);
        assert_eq!(super::next_power_of_two(1.0), 1.0);
        assert_eq!(super::next_power_of_two(1024.9), 1024.0);
        assert_eq!(super::next_power_of_two(1025.0), 2048.0);
    }
}
