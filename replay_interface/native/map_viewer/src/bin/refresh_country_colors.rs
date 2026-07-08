//! Refresh a bundle's `country_colors.json` from the installed content, applying the
//! current NAME1 + NAME1b cosmetic-color laws, WITHOUT regenerating the rest of
//! the bundle. Use after a cosmetic-law change (e.g. NAME1b DLC-conditional
//! cosmetics) so the deployed bundle picks up the new colours while every other
//! bundle artifact (terrain chunks, borders, rasters) stays byte-identical.
//!
//! Usage:
//!   refresh_country_colors --game <the game dir> [--mod <content dir> ...] \
//!       --bundle <bundle dir>
//!
//! Colours are RAW (pre-COL3 bake), matching what `country_colors.json`
//! already stores; the replay bakes them at render time.

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use map_viewer::{country_colors_from_game_resolver, GameFileResolver, Result};

fn main() -> Result<()> {
    let mut game = None;
    let mut mods = Vec::new();
    let mut bundle = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--bundle" => bundle = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                eprintln!(
                    "refresh_country_colors --game <the game dir> [--mod <dir> ...] --bundle <dir>"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <the game dir>")?;
    let bundle = bundle.ok_or("missing --bundle <bundle dir>")?;
    let target = bundle.join("country_colors.json");
    if !target.is_file() {
        return Err(format!("no country_colors.json in {}", bundle.display()).into());
    }

    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };
    let colors = country_colors_from_game_resolver(&resolver)?;

    // Diff against the existing file so the change is auditable.
    let previous: BTreeMap<String, [u8; 3]> =
        serde_json::from_slice(&std::fs::read(&target)?).unwrap_or_default();
    let mut changed = 0usize;
    for (tag, color) in &colors {
        if previous.get(tag) != Some(color) {
            println!(
                "{tag}: {:?} -> {:?}",
                previous.get(tag).copied().unwrap_or([0, 0, 0]),
                color
            );
            changed += 1;
        }
    }

    let json = serde_json::to_vec_pretty(&colors)?;
    std::fs::write(&target, json)?;
    println!(
        "wrote {} colours ({} changed) to {}",
        colors.len(),
        changed,
        target.display()
    );
    Ok(())
}
