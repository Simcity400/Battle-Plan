//! CHUNK1 — offline terrain-chunk mesh/LOD/CB1 generator, corpus byte-exact gate.
//!
//! Loads a captured `terrain_chunk_library.json` from a session directory,
//! regenerates every terrain-chunk draw's slot0 / slot1 window / index / CB1
//! from `map/heightmap.bmp` + the shipped vertex law (via the mod-aware
//! resolver), and reports byte-identity per draw and per LOD.
//!
//! Usage:
//!   dump_terrain_mesh_law --game <the game dir> --capture <session dir> [--mod <dir>]... [--verbose]

use map_viewer::{
    terrain_mesh_law_validate_corpus_with_resolver, GameFileResolver,
    NativeD3d11TerrainChunkLibrary, Result,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut capture = None;
    let mut mods: Vec<PathBuf> = Vec::new();
    let mut verbose = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--capture" => capture = args.next().map(PathBuf::from),
            "--mod" => {
                if let Some(dir) = args.next() {
                    mods.push(PathBuf::from(dir));
                }
            }
            "--verbose" => verbose = true,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }

    let game = game.ok_or("missing --game <game dir>")?;
    let capture = capture.ok_or("missing --capture <session dir>")?;
    let label = capture
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("capture")
        .to_string();

    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game.clone())?
    } else {
        GameFileResolver::with_mod_paths(game.clone(), mods)?
    };
    // The corpus gate only reads captured VB/IB/CB buffers + the shipped
    // heightmap, so records-only loading is enough and lets it run against
    // lightweight quick-capture harness runs that omit texture sidecars.
    let library = NativeD3d11TerrainChunkLibrary::load_records_only(&capture)?;
    let report = terrain_mesh_law_validate_corpus_with_resolver(&resolver, &library, &label)?;

    println!(
        "CORPUS {}: {} draws checked, {} byte-identical",
        report.session_label, report.draws_checked, report.draws_identical
    );
    for lod in &report.per_lod {
        println!(
            "  LOD tile_size={:>4}: {} checked, {} identical",
            lod.tile_size, lod.draws_checked, lod.draws_identical
        );
    }
    let mismatches = report
        .comparisons
        .iter()
        .filter(|c| !c.identical())
        .collect::<Vec<_>>();
    if verbose || !mismatches.is_empty() {
        for c in &report.comparisons {
            if verbose || !c.identical() {
                println!(
                    "  chunk {} size={} min=({},{}) slot1_off={} slot0={} slot1={} index={} cb1={} first_mismatch={:?}",
                    c.chunk_id,
                    c.tile_size,
                    c.min_x,
                    c.min_z,
                    c.slot1_offset,
                    c.slot0_identical,
                    c.slot1_identical,
                    c.index_identical,
                    c.cb1_identical,
                    c.first_mismatch
                );
            }
        }
    }

    if report.all_identical() {
        println!("VERDICT: BYTE-EXACT (all {} draws)", report.draws_checked);
        Ok(())
    } else {
        Err(format!(
            "VERDICT: NOT byte-exact — {} of {} draws mismatch",
            report.draws_checked - report.draws_identical,
            report.draws_checked
        )
        .into())
    }
}

fn print_usage() {
    println!(
        "Usage: dump_terrain_mesh_law --game <the game dir> --capture <session dir> [--mod <dir>]... [--verbose]"
    );
}
