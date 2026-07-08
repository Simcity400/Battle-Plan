//! OFF-5 probe: inventory every colorcorrection LUT (dimensions, hash,
//! identity deviation) through the resolver, and demo the exact
//! lut_blender CPU port on two cubes.
//!
//! Usage: dump_color_luts --game <the game dir> [--mod <content dir>]...

use map_viewer::{
    color_correction_lut_from_resolver, lut_blend1, scan_color_correction_luts_from_resolver,
    GameFileResolver, Result,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--help" | "-h" => {
                println!("Usage: dump_color_luts --game <the game dir> [--mod <dir>]...");
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
    let rows = scan_color_correction_luts_from_resolver(&resolver)?;
    println!(
        "{} colorcorrection LUTs (dims / slice / fnv1a64 / identity deviation 0..255):",
        rows.len()
    );
    for row in &rows {
        println!(
            "  {:44} {:4}x{:2} slice {:2} fnv {} identity_dev {:6.1}",
            row.rel_path,
            row.width,
            row.height,
            row.slice_size,
            row.fnv1a64,
            row.identity_deviation
        );
    }
    // Demo the exact LutBlend1 law: neutral -> winter at f2 = 0.5.
    let neutral =
        color_correction_lut_from_resolver(&resolver, "gfx/world/colorcorrection_neutral.tga")?;
    let winter =
        color_correction_lut_from_resolver(&resolver, "gfx/world/colorcorrection_winter.tga")?;
    let blended = lut_blend1(&neutral, &winter, 0.5)?;
    let probe = [0.5f32, 0.5, 0.5];
    println!(
        "LutBlend1(neutral, winter, 0.5) at rgb {probe:?}: neutral {:?} winter {:?} blended {:?}",
        neutral.sample_nearest(probe),
        winter.sample_nearest(probe),
        blended.sample_nearest(probe)
    );
    Ok(())
}
