//! Debug evidence tool for the pdxasset decoder (OFF-1).
//!
//! Modes:
//! * default: parse one `.mesh` (resolver-routed) and write a Wavefront OBJ
//!   so a human can look at the geometry.
//! * `--scan`: parse every `.mesh`/`.anim` under gfx/models (and `dlc/` when
//!   present), print the gate summary + property inventory.
//!
//! All game-file access goes through `GameFileResolver`
//! (MOD_COMPATIBILITY.md — the one rule); pass repeated `--mod` layers to
//! resolve against a modded recording.

use map_viewer::{
    pdx_model_from_resolver, pdx_model_to_obj, scan_pdx_assets_from_resolver, validate_pdx_model,
    GameFileResolver, Result, PDX_MODELS_ROOT,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut mesh = "gfx/models/buildings/bunker.mesh".to_string();
    let mut out = None;
    let mut scan = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => {
                mods.push(PathBuf::from(
                    args.next().ok_or("--mod needs a content dir")?,
                ));
            }
            "--mesh" => mesh = args.next().ok_or("--mesh needs a rel path")?,
            "--out" => out = args.next().map(PathBuf::from),
            "--scan" => scan = true,
            "--help" | "-h" => {
                print_usage();
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

    if scan {
        run_scan(&resolver)?;
        return Ok(());
    }

    let model = pdx_model_from_resolver(&resolver, &mesh)?;
    println!(
        "{mesh}: {} shape(s), {} locator(s), lod_distances {:?}",
        model.shapes.len(),
        model.locators.len(),
        model.lod_distances
    );
    for row in validate_pdx_model(&model) {
        println!(
            "  shape {} mesh {}: verts {} tris {} max_index {} aabb_dev {:.6} uv [{:?}, {:?}] {}",
            row.shape,
            row.mesh_index,
            row.vert_count,
            row.triangle_count,
            row.max_index,
            row.max_aabb_deviation,
            row.uv_min,
            row.uv_max,
            row.material_shader
        );
    }
    let out = out.ok_or("missing --out <obj path>")?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, pdx_model_to_obj(&model, &mesh))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn run_scan(resolver: &GameFileResolver) -> Result<()> {
    let mut roots = vec![PDX_MODELS_ROOT.to_string()];
    if resolver
        .resolve_dir_files_recursive("dlc", Some("mesh"))
        .is_ok()
    {
        roots.push("dlc".to_string());
    }
    let mut totals = (0usize, 0usize);
    for root in roots {
        let report = scan_pdx_assets_from_resolver(resolver, &root)?;
        totals.0 += report.mesh_files;
        totals.1 += report.anim_files;
        println!(
            "scan {root}: {} mesh + {} anim ({} binary, {} text); mesh failures {}; anim \
             failures {}; {} validation rows, {} semantic failures",
            report.mesh_files,
            report.anim_files,
            report.binary_files,
            report.text_files,
            report.mesh_failures.len(),
            report.anim_failures.len(),
            report.validation_rows,
            report.validation_failures.len()
        );
        for (rel, err) in report
            .mesh_failures
            .iter()
            .chain(report.anim_failures.iter())
            .chain(report.validation_failures.iter())
        {
            println!("  FAIL {rel}: {err}");
        }
        println!(
            "  property inventory ({} patterns):",
            report.property_inventory.len()
        );
        for ((pattern, name, type_char), count) in &report.property_inventory {
            println!("    {count:8} {type_char} {pattern} :: {name}");
        }
    }
    println!(
        "scan total: {} mesh + {} anim files parsed",
        totals.0, totals.1
    );
    Ok(())
}

fn print_usage() {
    println!(
        "Usage: dump_pdx_mesh --game <the game dir> [--mod <content dir>]... \
         [--mesh <rel .mesh path>] --out <obj path>\n       dump_pdx_mesh --game <the game dir> --scan"
    );
}
