use map_viewer::{
    compare_terrain_id_map_to_capture, dump_terrain_id_map_comparison_bmps, GameFileResolver,
    Result,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut capture = None;
    let mut out = None;
    let mut compare_only = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--capture" => capture = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--compare-only" => compare_only = true,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument {other}").into());
            }
        }
    }

    let game = game.ok_or("missing --game <game dir>")?;
    let capture = capture.ok_or("missing --capture <capture dir>")?;
    let resolver = GameFileResolver::vanilla(game.clone())?;
    let report = compare_terrain_id_map_to_capture(&resolver, &capture)?;
    println!(
        "TerrainIDMap verdict: byte_exact={} byte_mismatches={} texel_mismatches={} ({:.8}%) channel_mismatches={:?} pattern={} generated_fnv={} captured_fnv={}",
        report.byte_exact,
        report.byte_mismatches,
        report.texel_mismatches,
        report.texel_mismatch_percent,
        report.channel_mismatches,
        report.spatial_pattern,
        report.generated_descriptor.fnv1a64,
        report.captured_descriptor.fnv1a64
    );
    println!(
        "Orientation: {} @({}, {}) captured={:?} flipped={:?} unflipped={:?}",
        report.orientation_evidence.label,
        report.orientation_evidence.x,
        report.orientation_evidence.y,
        report.orientation_evidence.captured,
        report.orientation_evidence.generated_flipped_y,
        report.orientation_evidence.generated_unflipped_y
    );

    if !compare_only {
        let out = out.ok_or("missing --out <dump dir>; pass --compare-only to skip BMP dumps")?;
        let paths = dump_terrain_id_map_comparison_bmps(&resolver, &capture, &out)?;
        for path in paths {
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn print_usage() {
    println!(
        "Usage: dump_terrain_id_map_synthesis --game <the game dir> --capture <capture dir> [--out <dump dir> | --compare-only]"
    );
}
