use super::{Compiled, Compiler};
use content_reader_layers::content::diagnostics::{Diagnostics as ContentDiagnostics, Once};
use content_reader_gui::pdxshader::program::{Program, Technique};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const ABI: &str = "pdxshader-14/dxc-1.9.0.5402-0d3ee6b55/hlsl2016/dx-layout/naga-30.0.1";

pub trait Compile: Send + Sync {
    fn compile(
        &self,
        source: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<Compiled, String>;
    fn retry(&self) {}
}
impl Compile for Compiler {
    fn compile(&self, p: &Program, t: &Technique, d: &[String]) -> Result<Compiled, String> {
        Compiler::compile(self, p, t, d)
    }
}
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

type Cached = Arc<Once<Result<Arc<Compiled>, String>>>;

static COMPILED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DISK_HITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn totals() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (COMPILED.load(Relaxed), DISK_HITS.load(Relaxed))
}

pub struct Cache {
    root: Option<PathBuf>,
    compiler: Arc<dyn Compile>,
    diagnostics: Mutex<Diagnostics>,
    entries: Mutex<BTreeMap<String, Cached>>,
    warning: Option<String>,
}

impl Default for Cache {
    fn default() -> Self {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("BattlePlan/shader-cache/v2"));
        let mut cache = Self::new(root, Arc::new(Runtime::default()));
        if cache.root.is_none() {
            let warning = "LOCALAPPDATA is unset: compiled shaders are not persisted".to_owned();
            cache.warn(warning.clone());
            cache.warning = Some(warning);
        }
        cache
    }
}

impl Cache {
    pub fn new(root: Option<PathBuf>, compiler: Arc<dyn Compile>) -> Self {
        Self {
            root,
            compiler,
            diagnostics: Mutex::new(Diagnostics::default()),
            entries: Mutex::new(BTreeMap::new()),
            warning: None,
        }
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics.lock().unwrap().clone()
    }
    pub fn retry_failures(&self) {
        self.compiler.retry();
        self.entries
            .lock()
            .unwrap()
            .retain(|_, entry| !matches!(entry.peek(), Some(Err(_))));
    }
    fn warn(&self, warning: String) {
        ContentDiagnostics::shader(warning.clone());
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
        for stage in [
            content_reader_gui::pdxshader::program::Stage::Vertex,
            content_reader_gui::pdxshader::program::Stage::Pixel,
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
        if let Some(warning) = &self.warning {
            ContentDiagnostics::shader(warning.clone());
        }
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
                    DISK_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            COMPILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                independent_draws: false,
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
        content_reader_gui::pdxshader::program::read_with(b"vanilla.shader", &|path| Some(match path {
            b"vanilla.shader" => b"Includes = { common.fxh }\nVertexShader = { MainCode V [[ float4 main() : SV_POSITION { return C; } ]] }\nPixelShader = { MainCode P [[ float4 main() : SV_TARGET { return C; } ]] }\nEffect Up { VertexShader = V PixelShader = P }\nEffect Unused { VertexShader = Missing PixelShader = Missing }\n".to_vec(),
            b"gfx/FX/common.fxh" => format!("Code [[ {include} ]]\n").into_bytes(),
            _ => Vec::new(),
        })).unwrap()
    }

    #[test]
    fn cache_warning_entries_replay_only_their_own_key_in_either_order() {
        let scratch = super::super::Scratch::new().unwrap();
        let blocked = scratch.0.join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let sources = [
            source("static const float4 C = 1;"),
            source("static const float4 C = 0;"),
        ];
        let read = |cache: &Cache, index: usize| {
            content_reader_layers::content::diagnostics::observe(|| {
                cache
                    .get(&sources[index], &sources[index].techniques[0], &[])
                    .unwrap();
            })
            .1
        };
        let alone: Vec<_> = (0..2)
            .map(|index| {
                read(
                    &Cache::new(Some(blocked.clone()), Arc::new(Count::default())),
                    index,
                )
            })
            .collect();
        assert!(alone.iter().all(ContentDiagnostics::has_shader_warnings));
        assert_ne!(alone[0], alone[1]);
        for order in [[0, 1], [1, 0]] {
            let cache = Cache::new(Some(blocked.clone()), Arc::new(Count::default()));
            for _ in 0..2 {
                for index in order {
                    assert_eq!(read(&cache, index), alone[index]);
                }
            }
        }
    }
    #[test]
    fn shader_compiles_once_then_disk_cache_survives_a_new_process_and_invalidates() {
        const CACHE_DIR: &str = "SYNTHETIC_SHADER_CACHE_TEST_DIR";
        if let Some(directory) = std::env::var_os(CACHE_DIR) {
            let compiler = Arc::new(Count::default());
            let cache = Cache::new(Some(PathBuf::from(directory)), compiler.clone());
            let source = source("static const float4 C = 1;");
            cache.get(&source, &source.techniques[0], &[]).unwrap();
            assert_eq!(compiler.0.load(Ordering::SeqCst), 0);
            assert_eq!(cache.diagnostics().disk_hits, 1);
            return;
        }
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
        let test = concat!(
            module_path!(),
            "::shader_compiles_once_then_disk_cache_survives_a_new_process_and_invalidates"
        );
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test.split_once("::").unwrap().1])
            .env(CACHE_DIR, &scratch.0)
            .output()
            .unwrap();
        let output = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success() && output.contains("1 passed"), "{output}");
        let second = Cache::new(Some(scratch.0.clone()), compiler.clone());
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
            let missing = scratch.0.join(installation).join("dxcompiler.dll");
            let Err(reason) = Compiler::at(&missing) else {
                panic!("a missing compiler must be refused");
            };
            assert_eq!(reason, "DXC not found: dxcompiler.dll is not beside the application");
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
                Err("effect \"vanilla.shader\", technique \"Up\": DXC not found: dxcompiler.dll is not beside the application".into())
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
                    && reason.contains("DXC not found:")
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
        assert!(failed.select("Up").is_err());
        assert!(cache.get(&source, &source.techniques[0], &[]).is_err());
        compiles.select("Up").unwrap();
        assert_eq!(compiler.compiled.0.load(Ordering::SeqCst), 1);
        cache.retry_failures();
        assert_eq!(compiler.retried.load(Ordering::SeqCst), 1);
        let again = super::super::library::Effect::new(source.clone(), vec![], cache.clone());
        again.select("Up").unwrap();
        assert!(!again.failed());
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
            content_reader_gui::pdxshader::program::read_with::<Vec<u8>>(b"missing.lua", &|_| None).unwrap();
        assert_eq!(source.dependencies, [b"missing.shader"]);
        assert!(source.warnings[0].contains("empty technique record"));
        let effect = super::super::library::Effect::new(Arc::new(source), vec![], cache.clone());
        assert!(effect.select("Up").unwrap().is_none());
        assert_eq!(compiler.0.load(Ordering::SeqCst), 0);
        assert!(cache.diagnostics().refused.is_empty());
    }
}
