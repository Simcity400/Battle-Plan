//! Persistent CPU program cache. GPU objects remain device-local in `gpu::Cache`.
use super::{Compiled, Compiler};
use installed_content_core::pdxshader::program::{Program, Technique};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

// Bump whenever source assembly, DXC flags, naga or reflection changes. This is part of
// the filename hash as well as the payload; old caches cannot silently survive an upgrade.
// `Compiler::at` refuses any DXC but `DXC_VERSION`, which this names.
const ABI: &str = "pdxshader-10/dxc-1.9.0.5402-0d3ee6b55/hlsl2016/dx-layout/naga-30.0.1";

/// A failure names the effect and technique, as [`Compiler::compile`] does.
pub trait Compile: Send + Sync {
    fn compile(
        &self,
        source: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<Compiled, String>;
    /// Forget a compiler that was not found, so the next compile looks for it again.
    fn retry(&self) {}
}
impl Compile for Compiler {
    fn compile(&self, p: &Program, t: &Technique, d: &[String]) -> Result<Compiled, String> {
        Compiler::compile(self, p, t, d)
    }
}
/// The bundled compiler, discovered at the first cache miss of this cache (one content load)
/// or after a retry ([`Cache::retry_failures`]). A compiler installed later is found by the next
/// load.
struct Runtime {
    discover: fn() -> Result<Compiler, String>,
    found: Mutex<Option<Arc<Result<Compiler, String>>>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            discover: Compiler::discover,
            found: Mutex::new(None),
        }
    }
}
impl Compile for Runtime {
    fn compile(&self, p: &Program, t: &Technique, d: &[String]) -> Result<Compiled, String> {
        let found = Arc::clone(
            self.found
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get_or_insert_with(|| Arc::new((self.discover)())),
        );
        match &*found {
            Ok(compiler) => compiler.compile(p, t, d),
            Err(reason) => Err(format!(
                "effect {:?}, technique {:?}: {reason}",
                p.effect, t.name
            )),
        }
    }
    fn retry(&self) {
        let mut found = self.found.lock().unwrap_or_else(|error| error.into_inner());
        if found.as_deref().is_some_and(Result::is_err) {
            *found = None;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub compiled: u64,
    pub disk_hits: u64,
    pub generated_bytes: u64,
    pub refused: BTreeMap<String, u64>,
    pub warnings: BTreeMap<String, u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    abi: String,
    key: String,
    checksum: String,
    program: Compiled,
}

type Cached = Arc<OnceLock<Result<Arc<Compiled>, String>>>;

pub struct Cache {
    root: Option<PathBuf>,
    compiler: Arc<dyn Compile>,
    diagnostics: Mutex<Diagnostics>,
    entries: Mutex<BTreeMap<String, Cached>>,
}

impl Default for Cache {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("HOI4Replay/shader-cache/v2"));
        let cache = Self::new(root, Arc::new(Runtime::default()));
        if cache.root.is_none() {
            cache.warn("LOCALAPPDATA is unset: compiled shaders are not persisted".into());
        }
        cache
    }
}

impl Cache {
    /// `None` disables persistence, not compilation. Callers may isolate the cache for tests.
    pub fn new(root: Option<PathBuf>, compiler: Arc<dyn Compile>) -> Self {
        Self {
            root,
            compiler,
            diagnostics: Mutex::new(Diagnostics::default()),
            entries: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics.lock().unwrap().clone()
    }
    /// Forget failed compiles and a compiler that was not found, so a repaired compiler serves
    /// the next request; compiled programs stay.
    pub fn retry_failures(&self) {
        self.compiler.retry();
        self.entries
            .lock()
            .unwrap()
            .retain(|_, entry| !matches!(entry.get(), Some(Err(_))));
    }
    fn warn(&self, warning: String) {
        *self
            .diagnostics
            .lock()
            .unwrap()
            .warnings
            .entry(warning)
            .or_default() += 1;
    }
    pub fn refuse(&self, reason: &str) {
        *self
            .diagnostics
            .lock()
            .unwrap()
            .refused
            .entry(reason.into())
            .or_default() += 1;
    }
    pub fn key(
        source: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<String, String> {
        let mut hash = Sha256::new();
        hash.update(ABI);
        hash.update(source.identity);
        // Include emitted source as well: callers constructing Program directly are supported.
        for stage in [
            installed_content_core::pdxshader::program::Stage::Vertex,
            installed_content_core::pdxshader::program::Stage::Pixel,
        ] {
            let text = source.hlsl(technique, stage, defines)?;
            hash.update((text.len() as u64).to_le_bytes());
            hash.update(text.as_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub fn get(
        &self,
        source: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<Arc<Compiled>, String> {
        let key = Self::key(source, technique, defines)?;
        let entry = self
            .entries
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        entry
            .get_or_init(|| self.load_or_compile(key, source, technique, defines))
            .clone()
    }
    fn load_or_compile(
        &self,
        key: String,
        source: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<Arc<Compiled>, String> {
        let path = self
            .root
            .as_ref()
            .map(|root| root.join(format!("{key}.json")));
        if let Some(path) = &path
            && let Ok(bytes) = std::fs::read(path)
        {
            if let Ok(entry) = serde_json::from_slice::<Entry>(&bytes) {
                let bytes = serde_json::to_vec(&entry.program).map_err(|e| e.to_string())?;
                if entry.abi == ABI
                    && entry.key == key
                    && entry.checksum == format!("{:x}", Sha256::digest(bytes))
                {
                    self.diagnostics.lock().unwrap().disk_hits += 1;
                    return Ok(Arc::new(entry.program));
                }
            }
            self.warn(format!(
                "invalid shader cache {}: recompiling",
                path.display()
            ));
        }
        let compiled = self.compiler.compile(source, technique, defines)?;
        {
            let mut stats = self.diagnostics.lock().unwrap();
            stats.compiled += 1;
            stats.generated_bytes +=
                (compiled.vertex.wgsl.len() + compiled.pixel.wgsl.len()) as u64;
        }
        if let Some(path) = path {
            let write = || -> Result<(), String> {
                let root = path.parent().unwrap();
                std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
                let checksum = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&compiled).map_err(|e| e.to_string())?)
                );
                let entry = Entry {
                    abi: ABI.into(),
                    key,
                    checksum,
                    program: compiled.clone(),
                };
                let temp = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
                std::fs::write(
                    &temp,
                    serde_json::to_vec(&entry).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let result = std::fs::rename(&temp, &path);
                if result.is_err() {
                    let _ = std::fs::remove_file(&temp);
                }
                result.map_err(|e| e.to_string())
            };
            if let Err(error) = write() {
                self.warn(format!("shader cache {}: {error}", path.display()));
            }
        }
        Ok(Arc::new(compiled))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Default)]
    pub(crate) struct Count(AtomicUsize);
    impl Compile for Count {
        fn compile(&self, _: &Program, _: &Technique, _: &[String]) -> Result<Compiled, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let shader = super::super::Shader {
                wgsl: "fixture".into(),
                buffers: vec![],
                textures: vec![],
                samplers: vec![],
                sampling_pairs: vec![],
                inputs: vec![],
                clock: Default::default(),
            };
            Ok(Compiled {
                vertex: shader.clone(),
                pixel: shader,
            })
        }
    }
    pub(crate) fn source(include: &str) -> Program {
        crate::tests::initialize_runtime_tables();
        installed_content_core::pdxshader::program::read_with(b"vanilla.shader", &|path| Some(match path {
            b"vanilla.shader" => b"Includes = { common.fxh }\nVertexShader = { MainCode V [[ float4 main() : SV_POSITION { return C; } ]] }\nPixelShader = { MainCode P [[ float4 main() : SV_TARGET { return C; } ]] }\nEffect Up { VertexShader = V PixelShader = P }\nEffect Unused { VertexShader = Missing PixelShader = Missing }\n".to_vec(),
            b"gfx/FX/common.fxh" => format!("Code [[ {include} ]]\n").into_bytes(),
            _ => Vec::new(),
        })).unwrap()
    }
    #[test]
    fn vanilla_compiles_once_then_disk_cache_survives_two_runs_and_invalidates() {
        let scratch = super::super::Scratch::new().unwrap();
        let compiler = Arc::new(Count(AtomicUsize::new(0)));
        let source = Arc::new(source("static const float4 C = 1;"));
        let first = Arc::new(Cache::new(Some(scratch.0.clone()), compiler.clone()));
        let effect = super::super::library::Effect::new(source.clone(), vec![], first.clone());
        assert_eq!(compiler.0.load(Ordering::SeqCst), 0, "load is lazy");
        assert!(effect.select("Absent").unwrap().is_none());
        effect.select("Up").unwrap();
        effect.select("Up").unwrap();
        assert_eq!(compiler.0.load(Ordering::SeqCst), 1);
        drop(effect);
        drop(first);
        let second = Arc::new(Cache::new(Some(scratch.0.clone()), compiler.clone()));
        let effect = super::super::library::Effect::new(source.clone(), vec![], second.clone());
        effect.select("Up").unwrap();
        assert_eq!(
            compiler.0.load(Ordering::SeqCst),
            1,
            "second process lifetime must read disk"
        );
        assert_eq!(second.diagnostics().disk_hits, 1);
        let changed = self::source("static const float4 C = 0;");
        second.get(&changed, &changed.techniques[0], &[]).unwrap();
        second
            .get(&source, &source.techniques[0], &["NEW_DEFINE".into()])
            .unwrap();
        assert_eq!(
            compiler.0.load(Ordering::SeqCst),
            3,
            "includes and defines invalidate independently"
        );
        let key = Cache::key(&source, &source.techniques[0], &[]).unwrap();
        let path = scratch.0.join(format!("{key}.json"));
        let mut entry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        entry["abi"] = "obsolete compiler ABI".into();
        std::fs::write(path, serde_json::to_vec(&entry).unwrap()).unwrap();
        let third = Cache::new(Some(scratch.0.clone()), compiler.clone());
        third.get(&source, &source.techniques[0], &[]).unwrap();
        assert_eq!(compiler.0.load(Ordering::SeqCst), 4);
        assert_eq!(third.diagnostics().warnings.len(), 1);
    }
    #[test]
    fn abi_names_the_pinned_compiler_and_a_missing_one_is_named() {
        assert!(super::super::DXC_VERSION.starts_with("1.9.0.5402 (0d3ee6b55"));
        assert!(ABI.contains("/dxc-1.9.0.5402-0d3ee6b55/"));
        let scratch = super::super::Scratch::new().unwrap();
        for installation in ["baseline-main", "lane-det-1"] {
            let missing = scratch
                .0
                .join(installation)
                .join("shader-tools/dxc/dxc.exe");
            let Err(reason) = Compiler::at(missing) else {
                panic!("a missing compiler must be refused");
            };
            assert_eq!(
                reason,
                "DXC not found; install shader-tools/dxc/dxc.exe or set BATTLE_PLAN_SHADER_COMPILER"
            );
        }
    }

    #[test]
    fn missing_compiler_names_and_counts_only_selected_technique() {
        struct Missing;
        impl Compile for Missing {
            fn compile(
                &self,
                _: &Program,
                _: &Technique,
                _: &[String],
            ) -> Result<Compiled, String> {
                Err("effect \"vanilla.shader\", technique \"Up\": DXC not found; install shader-tools/dxc/dxc.exe or set BATTLE_PLAN_SHADER_COMPILER".into())
            }
        }
        let cache = Arc::new(Cache::new(None, Arc::new(Missing)));
        let effect = super::super::library::Effect::new(
            Arc::new(source("static const float4 C = 1;")),
            vec![],
            cache.clone(),
        );
        for _ in 0..2 {
            let reason = effect.select("Up").unwrap_err();
            assert!(
                reason.contains("vanilla.shader")
                    && reason.contains("Up")
                    && reason.contains("DXC not found;")
            );
        }
        assert_eq!(cache.diagnostics().refused.values().sum::<u64>(), 2);
    }
    #[test]
    fn a_compiler_repaired_after_a_failure_serves_once_failures_are_retried() {
        use std::sync::atomic::AtomicBool;
        #[derive(Default)]
        struct Repairable {
            repaired: AtomicBool,
            compiled: Count,
            retried: AtomicUsize,
        }
        impl Compile for Repairable {
            fn compile(&self, p: &Program, t: &Technique, d: &[String]) -> Result<Compiled, String> {
                if self.repaired.load(Ordering::SeqCst) {
                    self.compiled.compile(p, t, d)
                } else {
                    Err("effect \"vanilla.shader\", technique \"Up\": DXC not found".into())
                }
            }
            fn retry(&self) {
                self.retried.fetch_add(1, Ordering::SeqCst);
            }
        }
        let compiler = Arc::new(Repairable::default());
        let cache = Arc::new(Cache::new(None, compiler.clone()));
        let source = Arc::new(source("static const float4 C = 1;"));
        let failed = super::super::library::Effect::new(source.clone(), vec![], cache.clone());
        let compiles = super::super::library::Effect::new(
            source.clone(),
            vec!["OTHER".into()],
            cache.clone(),
        );
        assert!(failed.select("Up").is_err());
        assert!(failed.failed() && !compiles.failed());
        compiler.repaired.store(true, Ordering::SeqCst);
        // Until retried, the failure stays: in the effect and in the cache.
        assert!(failed.select("Up").is_err());
        assert!(cache.get(&source, &source.techniques[0], &[]).is_err());
        compiles.select("Up").unwrap();
        assert_eq!(compiler.compiled.0.load(Ordering::SeqCst), 1);
        cache.retry_failures();
        assert_eq!(compiler.retried.load(Ordering::SeqCst), 1);
        let again = super::super::library::Effect::new(source.clone(), vec![], cache.clone());
        again.select("Up").unwrap();
        assert!(!again.failed());
        // What compiled stays compiled.
        compiles.select("Up").unwrap();
        cache.get(&source, &source.techniques[0], &["OTHER".into()]).unwrap();
        assert_eq!(compiler.compiled.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_compiler_not_found_is_looked_for_again_only_after_a_retry() {
        static LOOKS: AtomicUsize = AtomicUsize::new(0);
        fn missing() -> Result<Compiler, String> {
            LOOKS.fetch_add(1, Ordering::SeqCst);
            Err("DXC not found".into())
        }
        let runtime = Runtime {
            discover: missing,
            found: Mutex::new(None),
        };
        let source = source("static const float4 C = 1;");
        for _ in 0..2 {
            let reason = runtime
                .compile(&source, &source.techniques[0], &[])
                .unwrap_err();
            assert!(reason.contains("DXC not found") && reason.contains("Up"), "{reason}");
        }
        assert_eq!(LOOKS.load(Ordering::SeqCst), 1);
        runtime.retry();
        assert!(
            runtime
                .compile(&source, &source.techniques[0], &[])
                .is_err()
        );
        assert_eq!(LOOKS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn variants_are_lazy_and_keep_pipeline_identity() {
        let compiler = Arc::new(Count(AtomicUsize::new(0)));
        let cache = Arc::new(Cache::new(None, compiler.clone()));
        let effect = super::super::library::Effect::new(
            Arc::new(source("static const float4 C = 1;")),
            vec![],
            cache,
        );
        let defines = vec!["ANIMATED".into(), "NUM_ANIMATIONS_2".into()];
        let variant = effect.variant(defines.clone());
        assert!(Arc::ptr_eq(&variant, &effect.variant(defines)));
        assert!(Arc::ptr_eq(&effect, &effect.variant(vec![])));
        assert_eq!(compiler.0.load(Ordering::SeqCst), 0);
        variant.select("Up").unwrap();
        variant.select("Up").unwrap();
        assert_eq!(compiler.0.load(Ordering::SeqCst), 1);
        effect.select("Up").unwrap();
        assert_eq!(compiler.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn absent_effect_is_the_null_record_without_compiler_or_header() {
        let compiler = Arc::new(Count(AtomicUsize::new(0)));
        let cache = Arc::new(Cache::new(None, compiler.clone()));
        let source =
            installed_content_core::pdxshader::program::read_with(b"missing.lua", &|_| None).unwrap();
        assert_eq!(source.dependencies, [b"missing.shader"]);
        assert!(source.warnings[0].contains("empty technique record"));
        let effect = super::super::library::Effect::new(Arc::new(source), vec![], cache.clone());
        assert!(effect.select("Up").unwrap().is_none());
        assert_eq!(compiler.0.load(Ordering::SeqCst), 0);
        assert!(cache.diagnostics().refused.is_empty());
    }
}
