use map_viewer::{
    compare_political_colors_to_capture, dump_political_color_comparison, GameFileResolver, Result,
};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut game = None;
    let mut capture = None;
    let mut out = None;
    let mut mods = Vec::new();
    let mut compare_only = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--capture" => capture = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--mod" => {
                let Some(path) = args.next() else {
                    return Err("missing path after --mod".into());
                };
                mods.push(PathBuf::from(path));
            }
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
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game.clone())?
    } else {
        GameFileResolver::with_mod_paths(game.clone(), mods)?
    };
    for warning in resolver.warnings() {
        eprintln!("resolver warning: {warning}");
    }

    if compare_only {
        let report = compare_political_colors_to_capture(&resolver, &capture)?;
        print_report(&report);
        return Ok(());
    }

    let out = out.ok_or("missing --out <dump dir>; pass --compare-only to skip dumps")?;
    let (report, paths) = dump_political_color_comparison(&resolver, &capture, &out)?;
    print_report(&report);
    println!("wrote {}", paths.synthesized_secondary_rgba.display());
    println!("wrote {}", paths.synthesized_owner_rgba.display());
    println!("wrote {}", paths.diff_heatmap_bmp.display());
    println!("wrote {}", paths.stats_json.display());
    Ok(())
}

fn print_report(report: &map_viewer::PoliticalColorComparisonReport) {
    println!(
        "ProvinceSecondaryColorMap verdict: byte_exact={} texel_match={:.8}% texel_mismatches={} byte_mismatches={} generated_fnv={} captured_fnv={} captured_all_zero={} vacuous_match={}",
        report.byte_exact,
        report.texel_match_percent,
        report.texel_mismatches,
        report.byte_mismatches,
        report.generated_descriptor.fnv1a64,
        report.captured_descriptor.fnv1a64,
        report.all_captured_texels_transparent_zero,
        report.validation_is_vacuous
    );
    if report.validation_is_vacuous {
        println!(
            "WARNING: vacuous match -- both synthesized and captured secondary maps are fully transparent; this does NOT validate the province-id -> occupation-stripe mapping"
        );
    }
    println!("Class counts:");
    for (class, count) in &report.class_counts {
        println!("  {class}: {count}");
    }
    println!("Mismatch class counts:");
    if report.mismatch_class_counts.is_empty() {
        println!("  none");
    } else {
        for (class, count) in &report.mismatch_class_counts {
            println!("  {class}: {count}");
        }
    }
    for note in &report.notes {
        println!("note: {note}");
    }
}

fn print_usage() {
    println!(
        "Usage: dump_political_colors --game <the game dir> --capture <capture dir> [--mod <content dir> ...] [--out <dump dir> | --compare-only]"
    );
}
