//! Offline compile census and synthetic native bloom ABI/port comparison.
//! Neither proves game-frame equivalence or every pass's native bindings.
//! Usage: map_shader_probe <report-dir> <base-game-folder> [ordered-overlay-folder...]

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use installed_content::{ResolvedLayers, parser, pdxshader::program, terrain};
use serde_json::json;

#[path = "../src/map/shader_sources.rs"]
mod shader_sources;

#[allow(dead_code)]
#[path = "../src/map/shader_scene.rs"]
mod shader_scene;

#[path = "../src/map/shader_constants.rs"]
mod shader_constants;

#[path = "map_shader_probe/bloom.rs"]
mod bloom;

#[path = "map_shader_probe/bindings.rs"]
mod bindings;

#[path = "map_shader_probe/reduce.rs"]
mod reduce;

fn overlay_shaders(root: &Path, directory: &Path, out: &mut BTreeSet<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            overlay_shaders(root, &entry.path(), out)?;
        } else if entry
            .path()
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("shader"))
        {
            out.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [output, roots @ ..] = args.as_slice() else {
        return Err(
            "usage: map_shader_probe <report-dir> <base-game-folder> [ordered-overlay-folder...]"
                .into(),
        );
    };
    if roots.is_empty() {
        return Err("a base game folder is required".into());
    }
    fs::create_dir_all(output)?;
    let mut replacements = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let descriptor = root.join("descriptor.mod");
        let paths = if index != 0 && descriptor.is_file() {
            let document = parser::parse(&fs::read(&descriptor)?)?;
            document
                .get_all("replace_path")
                .map(|value| {
                    value.scalar_str().map(str::to_owned).ok_or_else(|| {
                        io::Error::other(format!(
                            "{}: non-scalar replace_path",
                            descriptor.display()
                        ))
                    })
                })
                .collect::<io::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        replacements.push(paths);
    }
    let refs: Vec<Vec<&str>> = replacements
        .iter()
        .map(|paths| paths.iter().map(String::as_str).collect())
        .collect();
    let folders: Vec<_> = roots
        .iter()
        .zip(&refs)
        .map(|(root, paths)| (root.as_path(), paths.as_slice()))
        .collect();
    eprintln!(
        "Indexing and hashing {} ordered content folder(s)",
        folders.len()
    );
    let layers = ResolvedLayers::from_folders(&folders)?;
    eprintln!("Content folders resolved; auditing shader dependencies");
    let mut diagnostics = Vec::new();
    // Divergence is the expected result for an overlay; retain the complete refusal.
    let source_refusal = shader_sources::check(&layers, &mut diagnostics)
        .err()
        .map(|error| error.to_string());
    fs::write(output.join("source-audit.txt"), diagnostics.join("\n"))?;
    let mut effects: BTreeSet<_> = shader_sources::passes(&layers)?
        .into_iter()
        .map(|(_, effect)| effect)
        .collect();
    for root in roots.iter().skip(1) {
        let gfx = root.join("gfx");
        if gfx.is_dir() {
            overlay_shaders(root, &gfx, &mut effects)?;
        }
    }
    let map = terrain::read_default_map(&layers)?;
    let provinces = installed_content::image::bmp::decode_rgb(&layers.read(&map.file("provinces")?)?)?;
    // 140A4DDE0 derives the dimensions from the province raster. The DX11 NPOT
    // branch supplies 1 for the four ratios; no vanilla map dimensions are embedded.
    let map_size = [provinces.width, provinces.height];
    let defines = shader_sources::globals(map_size);
    drop(provinces);
    let cache = shader_pipeline::cache::Cache::default();
    let gpu = bindings::Probe::new().map_err(io::Error::other)?;
    let mut rows = Vec::new();
    let mut failures = 0;
    let mut binding_failures = 0;
    for (source_index, effect) in effects.iter().enumerate() {
        let source = match program::read(&layers, effect.as_bytes()) {
            Ok(source) => source,
            Err(reason) => {
                failures += 1;
                rows.push(json!({"effect": effect, "source_error": reason}));
                continue;
            }
        };
        fs::write(
            output.join(format!("source-{source_index:03}.txt")),
            format!("{source:#?}"),
        )?;
        for technique in &source.techniques {
            let mut variants = vec![defines.clone()];
            // 140FF1410 registers the alpha-read seed variant separately.
            if technique.name == "LuminanceDownsample" {
                let mut alpha = defines.clone();
                alpha.push("LUMINANCE_SAMPLE_ALPHA".into());
                variants.push(alpha);
            }
            // 140FF1410 registers all four HDR/COLOR_LUT caller combinations;
            // the native string bytes are at14296E608 and14296E610.
            if effect == "gfx/FX/restorescene.shader" {
                for extra in [&["HDR"][..], &["COLOR_LUT"][..], &["HDR", "COLOR_LUT"][..]] {
                    let mut variant = defines.clone();
                    variant.extend(extra.iter().map(|value| (*value).to_owned()));
                    variants.push(variant);
                }
            }
            for defines in variants {
                let started = std::time::Instant::now();
                let result = cache.get(&source, technique, &defines);
                let mut row = json!({"effect": effect, "technique": technique.name, "defines": defines,
                    "dependencies": source.dependencies, "warnings": source.warnings, "compile_ms": started.elapsed().as_secs_f64()*1000.0});
                match result {
                    Ok(compiled) => {
                        let index = rows.len();
                        fs::write(
                            output.join(format!("compiled-{index:03}-vertex.wgsl")),
                            &compiled.vertex.wgsl,
                        )?;
                        fs::write(
                            output.join(format!("compiled-{index:03}-pixel.wgsl")),
                            &compiled.pixel.wgsl,
                        )?;
                        row["compiled"] = json!(true);
                        row["gpu_abi"] = match gpu.bind(&compiled, &source, technique) {
                            Ok(report) => report,
                            Err(reason) => {
                                binding_failures += 1;
                                json!({"pipeline_and_bindings_created": false, "error": reason})
                            }
                        };
                        row["cache_key"] = json!(shader_pipeline::cache::Cache::key(
                            &source, technique, &defines
                        )?);
                        row["reflection"] = json!({"vertex": {
                            "buffers": compiled.vertex.buffers, "textures": compiled.vertex.textures,
                            "samplers": compiled.vertex.samplers, "inputs": compiled.vertex.inputs},
                            "pixel": {"buffers": compiled.pixel.buffers, "textures": compiled.pixel.textures,
                            "samplers": compiled.pixel.samplers}});
                    }
                    Err(reason) => {
                        failures += 1;
                        row["compiled"] = json!(false);
                        row["error"] = json!(reason);
                        for stage in [program::Stage::Vertex, program::Stage::Pixel] {
                            if let Ok(hlsl) = source.hlsl(technique, stage, &defines) {
                                fs::write(
                                    output.join(format!("failed-{:03}-{stage:?}.hlsl", rows.len())),
                                    hlsl,
                                )?;
                            }
                        }
                    }
                }
                println!("{} {}: {}", effect, technique.name, row["compiled"]);
                rows.push(row);
            }
        }
    }
    let bloom = match bloom::run(&layers, &defines, &cache, output) {
        Ok(report) => report,
        Err(error) => {
            failures += 1;
            json!({"error":error.to_string()})
        }
    };
    let cache = cache.diagnostics();
    let reductions = match reduce::run(&layers, map_size, output) {
        Ok(report) => report,
        Err(error) => {
            failures += 1;
            json!({"error": error.to_string()})
        }
    };
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({"ordered_folders": roots,
        "scope": "offline explicitly ordered folder stack; not a recorded identity or native-binding proof",
        "source_refusal": source_refusal, "native_bindings_verified": false, "gpu_rendered": false,
        "bloom_fixture": bloom,
        "reduction_fixture": reductions,
        "gpu_abi": {"adapter": gpu.adapter, "failures": binding_failures,
            "native_inputs": false},
        "cache": {"compiled": cache.compiled, "disk_hits": cache.disk_hits, "warnings": cache.warnings},
        "failures": failures, "results": rows}))?,
    )?;
    println!(
        "{} source files; {failures} failed source/technique variants",
        effects.len()
    );
    if failures != 0 || binding_failures != 0 {
        return Err(format!("{failures} shader census failures, {binding_failures} GPU ABI failures; see report.json").into());
    }
    Ok(())
}
