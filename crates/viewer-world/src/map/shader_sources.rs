
#[cfg(test)]
use std::path::PathBuf;
use std::{cell::RefCell, collections::BTreeMap, collections::BTreeSet, io, sync::Arc};

use content_reader_core::{ResolvedLayers, pdxshader};
use content_reader_image::native::Decode;

pub fn globals(map: [u32; 2], decode: Decode) -> Vec<String> {
    shader_pipeline::library::globals(map, decode)
}

pub fn library(
    layers: &ResolvedLayers,
    map: [u32; 2],
    decode: Decode,
) -> (shader_pipeline::library::Library, Vec<String>) {
    let defines = globals(map, decode);
    (
        shader_pipeline::library::Library::new(Arc::new(layers.clone())),
        defines,
    )
}

pub const PASSES: &[(&str, &str)] = &[
    ("terrain", "gfx/FX/pdxmap.shader"),
    ("terrain refraction", "gfx/FX/pdxmap.shader"),
    ("terrain shadow receiver", "gfx/FX/pdxmap.shader"),
    ("land borders", "gfx/FX/border.shader"),
    ("sea borders", "gfx/FX/border.shader"),
    ("demilitarized borders", "gfx/FX/border.shader"),
    ("straits", "gfx/FX/strait.shader"),
    ("rivers", "gfx/FX/river.shader"),
    ("railways", "gfx/models/supply/railroad.shader"),
    ("trees", "gfx/FX/tree.shader"),
    ("tree shadow caster", "gfx/FX/tree.shader"),
    ("tree shadow receiver", "gfx/FX/tree.shader"),
    ("water", "gfx/FX/pdxwater.shader"),
    ("sky", "gfx/FX/sky.shader"),
    ("map frame", "gfx/FX/pdxmesh.shader"),
    ("city sprawl", "gfx/FX/pdxmesh.shader"),
    ("map buildings", "gfx/FX/pdxmesh.shader"),
    ("unit models", "gfx/FX/pdxmesh.shader"),
    ("bridges", "gfx/FX/pdxmesh.shader"),
    ("aircraft", "gfx/FX/pdxmesh.shader"),
    ("trains", "gfx/FX/pdxmesh.shader"),
    ("mesh shadow casters", "gfx/FX/pdxmesh.shader"),
    ("country names", "gfx/FX/mapname.shader"),
    ("weather and entity particles", "gfx/FX/particle.shader"),
    ("shadow blur horizontal", "gfx/FX/shadowblur.shader"),
    ("shadow blur vertical", "gfx/FX/shadowblur.shader"),
    ("bloom seed", "gfx/FX/downsample.shader"),
    ("luminance reductions", "gfx/FX/downsample_luminance.shader"),
    (
        "luminance gather and adaptation",
        "gfx/FX/downsample_luminance.shader",
    ),
    ("bloom horizontal", "gfx/FX/bloom.shader"),
    ("bloom vertical", "gfx/FX/bloom.shader"),
    ("scene restore", "gfx/FX/restorescene.shader"),
    ("colour LUT blending", "gfx/FX/lut_blender.shader"),
];

type Sources<B> = BTreeMap<String, Option<B>>;

#[derive(Debug)]
struct Graph<B> {
    sources: Sources<B>,
    errors: BTreeSet<String>,
}

impl<B> Default for Graph<B> {
    fn default() -> Self { Self { sources: BTreeMap::new(), errors: BTreeSet::new() } }
}

fn graph<B: Clone + AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_core::content::Bytes>>(effect: &str, read: &dyn Fn(&str) -> io::Result<B>) -> Graph<B> {
    let out = RefCell::new(Graph::default());
    let read = |path: &str| {
        if let Some(bytes) = out.borrow().sources.get(path) {
            return bytes.clone();
        }
        let bytes = match read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                out.borrow_mut().errors.insert(format!("{path}: {error}"));
                None
            }
        };
        out.borrow_mut().sources.insert(path.into(), bytes.clone());
        bytes
    };
    let read_bytes = |path: &[u8]| match std::str::from_utf8(path) {
        Ok(path) => read(path),
        Err(_) => {
            out.borrow_mut().errors.insert(format!(
                "{}: a non-UTF-8 include path this audit cannot key",
                String::from_utf8_lossy(path)
            ));
            None
        }
    };
    if pdxshader::load_with(effect.as_bytes(), &read_bytes).is_none() {
        out.borrow_mut()
            .errors
            .insert(format!("missing shader {effect}"));
    }
    let header = "gfx/FX/defines_hlsl_dx11.fxh";
    if read(header).is_none() {
        out.borrow_mut()
            .errors
            .insert(format!("missing shader header {header}"));
    }
    out.into_inner()
}

#[derive(Debug, Default)]
pub struct Audit {
    pub lines: Vec<String>,
    pub affected_passes: usize,
}

fn audit_paths<B: Clone + Eq + AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_core::content::Bytes>>(
    layers: &ResolvedLayers,
    passes: &[(&str, &str, &str)],
    vanilla: &dyn Fn(&str) -> io::Result<B>,
    content: &dyn Fn(&str) -> io::Result<B>,
) -> Audit {
    let mut graphs = BTreeMap::new();
    let mut changed_files = BTreeSet::new();
    let mut out = Audit::default();
    for &(pass, base_effect, effect) in passes {
        let (base, resolved) = graphs
            .entry((base_effect, effect))
            .or_insert_with(|| (graph(base_effect, vanilla), graph(effect, content)));
        let paths: BTreeSet<_> = base.sources.keys().chain(resolved.sources.keys()).collect();
        let changed: Vec<_> = paths
            .into_iter()
            .filter(|path| base.sources.get(*path) != resolved.sources.get(*path))
            .cloned()
            .collect();
        let errors: Vec<_> = base.errors.union(&resolved.errors).cloned().collect();
        if changed.is_empty() && errors.is_empty() {
            continue;
        }
        out.affected_passes += 1;
        let status =
            "compiled content shader; binding and compilation failures are reported by the pass";
        changed_files.extend(changed.iter().cloned());
        out.lines.push(format!(
            "Map shader pass {pass:?} ({effect}): {} differing source(s) [{}]{}; {status}.",
            changed.len(),
            changed.join(", "),
            if errors.is_empty() {
                String::new()
            } else {
                format!("; {} source error(s): {}", errors.len(), errors.join("; "))
            },
        ));
    }
    let uncovered: BTreeSet<_> = graphs
        .values()
        .flat_map(|(base, resolved)| base.sources.iter().chain(&resolved.sources))
        .filter(|(path, bytes)| bytes.is_some() && !layers.covered(path))
        .map(|(path, _)| path.as_str())
        .collect();
    out.lines.insert(0, format!(
        "Map shader source audit: {}/{} affected passes, {} distinct differing source/include file(s); all map passes use compiled content shaders; comparison is with the installed base game. {} source/include file(s) are outside recorded content hash coverage and are not hash-verified{}",
        out.affected_passes, passes.len(), changed_files.len(), uncovered.len(),
        if uncovered.is_empty() {
            ".".to_owned()
        } else {
            format!(": {}.", uncovered.into_iter().collect::<Vec<_>>().join(", "))
        },
    ));
    out
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn passes(layers: &ResolvedLayers) -> io::Result<Vec<(String, String)>> {
    passes_with(&|path| layers.read_uncovered(path))
}

fn passes_with<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]>>(read: &dyn Fn(&str) -> io::Result<B>) -> io::Result<Vec<(String, String)>> {
    let mut passes: Vec<_> = PASSES
        .iter()
        .map(|&(name, path)| (name.into(), path.into()))
        .collect();
    let path = "gfx/maparrows/maparrows.txt";
    let document = content_reader_core::parser::parse(&read(path)?)?;
    let effect = |items: &[content_reader_core::parser::Item], kind: &str| -> io::Result<String> {
        let value = items.iter().rev().find(|item| item.key.as_deref().is_some_and(|key| key.eq_ignore_ascii_case(b"effect")))
            .and_then(|item| item.value.scalar())
            .ok_or_else(|| invalid(format!("{path}: {kind} has no scalar effect; shader source audit cannot establish its source")))?;
        String::from_utf8(pdxshader::effect_path(value)).map_err(|path| {
            invalid(format!(
                "{path}: {kind}'s effect {:?} is not UTF-8; shader source audit cannot key it",
                String::from_utf8_lossy(path.as_bytes())
            ))
        })
    };
    passes.push(("map arrows".into(), effect(&document.items, "arrows")?));
    let text = document
        .items
        .iter()
        .rev()
        .find(|item| {
            item.key
                .as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case(b"arrowText"))
        })
        .and_then(|item| item.value.items())
        .ok_or_else(|| {
            invalid(format!(
                "{path}: no arrowText block; shader source audit cannot establish its source"
            ))
        })?;
    passes.push(("order labels".into(), effect(text, "arrowText")?));
    Ok(passes)
}

pub fn check(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> io::Result<()> {
    let vanilla = layers.base_only()?;
    let base_passes = passes(&vanilla)?;
    let content_passes = passes(layers)?;
    let passes: Vec<_> = base_passes
        .iter()
        .zip(&content_passes)
        .map(|((name, base), (_, path))| (name.as_str(), base.as_str(), path.as_str()))
        .collect();
    let audit = audit_paths(
        layers,
        &passes,
        &|path| vanilla.read_uncovered(path),
        &|path| layers.read_uncovered(path),
    );
    warnings.extend(audit.lines.iter().cloned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_globals_follow_the_clients_force_pow2_textures() {
        let on = Decode {
            force_pow2_textures: true,
        };
        let has = |defines: &[String], define: &str| defines.iter().any(|d| d == define);
        let off = globals([5632, 2048], Decode::default());
        assert!(has(&off, "MAP_POW2_X 1.000") && has(&off, "FOW_POW2_X 1.000"));
        let (_, defines) = library(&ResolvedLayers::default(), [5632, 2048], on);
        assert!(has(&defines, "MAP_POW2_X 0.688") && has(&defines, "FOW_POW2_X 0.917"));
        assert_eq!(defines, shader_pipeline::library::globals([5632, 2048], on));
    }

    #[test]
    #[ignore = "reads the installed game's shaders; set BATTLE_PLAN_GAME_DIR"]
    fn installed_map_passes_point_sample_the_light_table() {
        use content_reader_core::{
            pdxshader::program, pdxshader::program::Filter, pdxshader::program::Stage,
        };
        let root = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap());
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let point = |sampler: &program::Sampler| {
            [sampler.mag, sampler.min, sampler.mip]
                .iter()
                .all(|filter| *filter == Filter::Point)
        };
        let mut declared = 0;
        for (pass, file) in passes(&layers).unwrap() {
            let source = program::read(&layers, file.as_bytes()).unwrap();
            for technique in &source.techniques {
                let state = shader_pipeline::gpu::State::authored(&source, technique).unwrap();
                for sampler in state.samplers.values() {
                    if sampler.name == "LightDataMap" {
                        declared += 1;
                        assert!(point(sampler), "{pass} {}: {sampler:?}", technique.name);
                    }
                }
            }
        }
        assert!(declared > 0);
        let physical = super::super::mesh_shader::map_samplers();
        assert!(point(&physical[&shader_pipeline::binding(9, Stage::Pixel)]));
    }

    fn audit_with(
        passes: &[(&str, &str)],
        vanilla: &dyn Fn(&str) -> io::Result<Vec<u8>>,
        content: &dyn Fn(&str) -> io::Result<Vec<u8>>,
    ) -> Audit {
        audit_paths(
            &ResolvedLayers::default(),
            &passes
                .iter()
                .map(|&(name, path)| (name, path, path))
                .collect::<Vec<_>>(),
            vanilla,
            content,
        )
    }

    fn source(files: &[(&str, &str)], path: &str) -> io::Result<Vec<u8>> {
        if path == "gfx/FX/defines_hlsl_dx11.fxh" {
            return Ok(Vec::new());
        }
        files
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, text)| text.as_bytes().to_vec())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    #[test]
    fn compatibility_policy3_shader_sources_keep_the_uncovered_read_note() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let layers = ResolvedLayers::from_folders_with_coverage_policy(&[], 3).unwrap();
        let files = [
            (
                "gfx/FX/synthetic.shader",
                "Includes = { \"synthetic.fxh\" }",
            ),
            ("gfx/FX/synthetic.fxh", ""),
        ];
        let passes = [(
            "synthetic",
            "gfx/FX/synthetic.shader",
            "gfx/FX/synthetic.shader",
        )];
        let read = |path: &str| source(&files, path);
        let audit = audit_paths(&layers, &passes, &read, &read);
        assert_eq!(audit.affected_passes, 0);
        assert!(audit.lines[0].contains("3 source/include file(s) are outside recorded content hash coverage and are not hash-verified"));
        for path in [
            "gfx/FX/synthetic.shader",
            "gfx/FX/synthetic.fxh",
            "gfx/FX/defines_hlsl_dx11.fxh",
        ] {
            assert!(audit.lines[0].contains(path));
        }
        let audit = audit_paths(&ResolvedLayers::default(), &passes, &read, &read);
        assert!(
            audit.lines[0]
                .contains("0 source/include file(s) are outside recorded content hash coverage")
        );
    }

    #[test]
    fn recursive_shared_include_counts_all_passes_once_and_names_files() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let base = [
            ("root", "Includes = { \"a.fxh\" }"),
            ("gfx/FX/a.fxh", "Includes = { \"b.fxh\" }"),
            ("gfx/FX/b.fxh", "Code [[ float x = 1; ]]"),
        ];
        let mut content = base;
        content[2].1 = "Includes = { \"a.fxh\" } Code [[ float x = 2; ]]";
        let passes = [("terrain", "root"), ("water", "root")];
        let audit = audit_with(&passes, &|p| source(&base, p), &|p| source(&content, p));
        assert_eq!(audit.affected_passes, 2);
        assert!(audit.lines[0].contains("2/2 affected passes, 1 distinct"));
        assert!(audit.lines[0].ends_with(
            "1 source/include file(s) are outside recorded content hash coverage and are not hash-verified: root."
        ));
        assert!(audit.lines[1].contains("terrain"));
        assert!(audit.lines[2].contains("water"));
        assert!(audit.lines[1..].iter().all(|l| l.contains("gfx/FX/b.fxh")));
        assert_eq!(
            audit_with(&passes, &|p| source(&base, p), &|p| source(&base, p)).affected_passes,
            0
        );
    }

    #[test]
    fn all_compiled_passes_report_changes_without_a_vanilla_only_gate() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let base = [
            ("root", "Includes = { \"shared.fxh\" }"),
            ("gfx/FX/shared.fxh", ""),
        ];
        let mut content = base;
        content[1].1 = "Code [[ float changed = 1; ]]";
        let passes = [
            ("bloom horizontal", "root"),
            ("bloom vertical", "root"),
            ("luminance reductions", "root"),
            ("country names", "root"),
            ("order labels", "root"),
            ("weather and entity particles", "root"),
        ];
        let audit = audit_with(&passes, &|p| source(&base, p), &|p| source(&content, p));
        assert_eq!(audit.affected_passes, 6);
        assert!(
            audit.lines[1..]
                .iter()
                .all(|line| line.contains("compiled content shader"))
        );
        let audit = audit_with(&passes[..2], &|p| source(&base, p), &|p| {
            source(&content, p)
        });
        assert_eq!(audit.affected_passes, 2);
    }

    #[test]
    fn removed_and_added_dependencies_and_hard_errors_are_not_silent() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let base = [
            ("root", "Includes = { \"old.fxh\" }"),
            ("gfx/FX/old.fxh", ""),
        ];
        let content = [
            ("root", "Includes = { \"new.fxh\" }"),
            ("gfx/FX/new.fxh", ""),
        ];
        let audit = audit_with(&[("test", "root")], &|p| source(&base, p), &|p| {
            source(&content, p)
        });
        assert_eq!(audit.affected_passes, 1);
        assert!(audit.lines[1].contains("3 differing source(s)"));
        for name in ["old.fxh", "new.fxh", "root"] {
            assert!(audit.lines[1].contains(name));
        }
        let audit = audit_with(&[("test", "root")], &|p| source(&base, p), &|_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(audit.lines[1].contains("source error(s)"));
        assert!(audit.lines[1].contains("missing shader root"));
    }

    #[test]
    fn authored_arrow_effect_paths_are_audited_including_replacements() {
        let passes = passes_with(&|_| {
            Ok(br#"
            effect = "gfx/FX/old.lua"
            effect = "gfx/FX/custom.lua"
            arrowText = { effect = "gfx/FX/custom_text.lua" }
        "#
            .to_vec())
        })
        .unwrap();
        assert_eq!(
            passes[passes.len() - 2],
            ("map arrows".into(), "gfx/FX/custom.shader".into())
        );
        assert_eq!(
            passes[passes.len() - 1],
            ("order labels".into(), "gfx/FX/custom_text.shader".into())
        );
        let audit = audit_paths(
            &ResolvedLayers::default(),
            &[("map arrows", "old.shader", "custom.shader")],
            &|p| source(&[("old.shader", "")], p),
            &|p| source(&[("custom.shader", "")], p),
        );
        assert_eq!(audit.affected_passes, 1);
        assert!(audit.lines[1].contains("old.shader"));
        assert!(audit.lines[1].contains("custom.shader"));
        assert!(
            passes_with(&|_| Ok(b"effect = {}".to_vec()))
                .unwrap_err()
                .to_string()
                .contains("no scalar effect")
        );
    }
}
