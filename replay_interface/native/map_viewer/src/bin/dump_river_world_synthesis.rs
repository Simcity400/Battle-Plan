//! OFF-2 evidence tool: synthesize the world river mesh from rivers.bmp +
//! heightmap.bmp (resolver-routed) and print the quantitative comparison
//! against one or more captured rivers-pass sessions.
//!
//! Usage:
//!   dump_river_world_synthesis --game <the game dir> [--mod <content dir>]...
//!       --capture <session dir> [--capture <session dir>]...

use map_viewer::{
    compare_river_world_mesh_to_capture, heightmap_bmp_from_resolver, rivers_bmp_from_resolver,
    synthesize_river_world_mesh, GameFileResolver, Result, RiverWorldSynthesisParams,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut captures = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--capture" => {
                captures.push(PathBuf::from(args.next().ok_or("--capture needs a dir")?))
            }
            "--help" | "-h" => {
                println!(
                    "Usage: dump_river_world_synthesis --game <the game dir> [--mod <dir>]... \
                     --capture <session dir>..."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    if captures.is_empty() {
        return Err("missing --capture <session dir> (repeatable)".into());
    }
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };
    let params = RiverWorldSynthesisParams::default();
    let rivers = rivers_bmp_from_resolver(&resolver)?;
    let heightmap = heightmap_bmp_from_resolver(&resolver)?;
    let mesh = synthesize_river_world_mesh(&rivers, &heightmap, &params)?;
    println!(
        "generated world river mesh: {} pairs, {} strips ({} from sources), {} river pixels ({} sourceless px skipped per engine law), class histogram {:?}",
        mesh.pair_count(),
        mesh.strips.len(),
        mesh.strips.iter().filter(|s| s.starts_at_source).count(),
        mesh.river_pixel_count,
        mesh.skipped_sourceless_pixels,
        mesh.class_histogram
    );
    for capture in captures {
        let report = compare_river_world_mesh_to_capture(&mesh, &rivers, &heightmap, &capture)?;
        report.print();
    }
    Ok(())
}
