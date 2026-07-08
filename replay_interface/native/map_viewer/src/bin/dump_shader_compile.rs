//! OFF-3 evidence tool: parse all shipped effect files, compile the
//! map-relevant effects, and print the reflection/hash comparison table
//! against a captured session.
//!
//! Usage:
//!   dump_shader_compile --game <the game dir> [--mod <dir>]... [--capture <session dir>]

use map_viewer::{scan_effect_files_from_resolver, GameFileResolver, Result};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut capture: Option<PathBuf> = None;
    let mut dump_hlsl: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--capture" => capture = args.next().map(PathBuf::from),
            "--dump-hlsl" => dump_hlsl = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!(
                    "Usage: dump_shader_compile --game <the game dir> [--mod <dir>]... [--capture \
                     <session dir>] [--dump-hlsl <dir>]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };

    let scan = scan_effect_files_from_resolver(&resolver)?;
    println!(
        "effect parse scan: {} .shader + {} effect-syntax .fxh files, {} effects, {} MainCode \
         blocks, {} samplers, {} failures",
        scan.shader_files,
        scan.fxh_files,
        scan.effects,
        scan.main_codes,
        scan.samplers,
        scan.failures.len()
    );
    for (rel, err) in &scan.failures {
        println!("  FAIL {rel}: {err}");
    }
    if !scan.failures.is_empty() {
        return Err("effect parse failures".into());
    }

    #[cfg(windows)]
    {
        use map_viewer::{
            build_stage_hlsl, compare_map_effects_to_capture, d3d_compile_stage, ShaderStage,
            MAP_EFFECT_COMPILE_ONLY,
        };
        // Compile-only coverage for effects without captured bundles.
        for (shader_rel, effects) in MAP_EFFECT_COMPILE_ONLY {
            for effect in *effects {
                for stage in [ShaderStage::Vertex, ShaderStage::Pixel] {
                    let hlsl = build_stage_hlsl(&resolver, shader_rel, effect, stage)?;
                    if let Some(dir) = &dump_hlsl {
                        fs::create_dir_all(dir)?;
                        let file_name = format!(
                            "{}__{}__{}.hlsl",
                            shader_rel.replace(['/', '\\'], "_"),
                            effect,
                            stage.profile()
                        );
                        fs::write(dir.join(file_name), &hlsl)?;
                    }
                    match d3d_compile_stage(&hlsl, stage, 0) {
                        Ok(blob) => {
                            // Write the compiled DXBC next to any --dump-hlsl dir so
                            // observed-shader bins can be byte/hash-compared offline.
                            if let Some(dir) = &dump_hlsl {
                                let bin_name = format!(
                                    "{}__{}__{}.dxbc",
                                    shader_rel.replace(['/', '\\'], "_"),
                                    effect,
                                    stage.profile()
                                );
                                fs::write(dir.join(bin_name), &blob)?;
                            }
                            println!(
                                "compile-only {shader_rel}:{effect} [{}] OK ({} bytes)",
                                stage.profile(),
                                blob.len()
                            )
                        }
                        Err(err) => println!(
                            "compile-only {shader_rel}:{effect} [{}] FAILED: {err}",
                            stage.profile()
                        ),
                    }
                }
            }
        }
        if let Some(capture) = capture {
            let rows = compare_map_effects_to_capture(&resolver, &capture)?;
            let mut matched = 0usize;
            for row in &rows {
                row.print();
                if row.reflection_matches() {
                    matched += 1;
                }
            }
            println!(
                "reflection comparison: {}/{} stage rows MATCH captured reflection",
                matched,
                rows.len()
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = capture;
        let _ = dump_hlsl;
        println!("D3DCompile comparison requires Windows");
    }
    Ok(())
}
