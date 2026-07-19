use game_ui::major_views::EXPECTED_MAJOR_VIEW_COUNT;
use game_ui::probe::{default_probe_config, run_probe};

fn main() {
    let config = default_probe_config();
    if !config.vanilla_root.is_dir() {
        println!(
            "SKIP game_ui acceptance: the game root missing at {}",
            config.vanilla_root.display()
        );
        return;
    }

    match run_probe(&config) {
        Ok(report) => {
            println!("{}", report.to_text());
            if report.gfx.parse_failures != 0 {
                eprintln!("gfx parse failures: {}", report.gfx.parse_failures);
                std::process::exit(1);
            }
            if report.gui.parse_failures != 0 {
                eprintln!("gui parse failures: {}", report.gui.parse_failures);
                std::process::exit(1);
            }
            if report.topbar.sprite_ref_count == 0 || report.topbar.resolved_texture_count == 0 {
                eprintln!("topbar gate did not find sprite references/textures");
                std::process::exit(1);
            }
            if !report.topbar.missing.is_empty() || !report.topbar.undecodable.is_empty() {
                eprintln!(
                    "topbar unresolved textures: missing={} undecodable={}",
                    report.topbar.missing.len(),
                    report.topbar.undecodable.len()
                );
                std::process::exit(1);
            }
            if report.topbar_composition.sample_missing_sprites != 0
                || report.topbar_composition.sample_undecodable_sprites != 0
                || !report.topbar_composition.sample_missing_glyphs.is_empty()
                || report.topbar_composition.empty_missing_sprites != 0
                || report.topbar_composition.empty_undecodable_sprites != 0
                || !report.topbar_composition.empty_missing_glyphs.is_empty()
            {
                eprintln!("topbar composition gate failed");
                std::process::exit(1);
            }
            if !report.hud_window.missing_sprites.is_empty()
                || !report.hud_window.undecodable_sprites.is_empty()
                || report.hud_composition.sample_missing_sprites != 0
                || report.hud_composition.sample_undecodable_sprites != 0
                || !report.hud_composition.sample_missing_glyphs.is_empty()
                || report.hud_composition.empty_missing_sprites != 0
                || report.hud_composition.empty_undecodable_sprites != 0
                || !report.hud_composition.empty_missing_glyphs.is_empty()
            {
                eprintln!("hud composition gate failed");
                std::process::exit(1);
            }
            if report.major_views.len() != EXPECTED_MAJOR_VIEW_COUNT {
                eprintln!(
                    "expected {} major view reports, got {}",
                    EXPECTED_MAJOR_VIEW_COUNT,
                    report.major_views.len()
                );
                std::process::exit(1);
            }
            for view in &report.major_views {
                if view.sample_drawn_elements == 0
                    || view.sample_missing_sprites != 0
                    || view.sample_undecodable_sprites != 0
                    || view.sample_missing_fonts != 0
                    || !view.sample_missing_glyphs.is_empty()
                    || !view.sample_unresolved_loc_keys.is_empty()
                    || view.documented_skips.is_empty()
                    || view.proof_png.is_none()
                {
                    eprintln!(
                        "major view gate failed for {}: drawn={} skips={} missing_sprites={} undecodable_sprites={} missing_fonts={} missing_glyph_sets={} unresolved_loc_keys={} png={}",
                        view.display_name,
                        view.sample_drawn_elements,
                        view.documented_skips.len(),
                        view.sample_missing_sprites,
                        view.sample_undecodable_sprites,
                        view.sample_missing_fonts,
                        view.sample_missing_glyphs.len(),
                        view.sample_unresolved_loc_keys.len(),
                        view.proof_png
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "<not written>".to_string())
                    );
                    std::process::exit(1);
                }
            }
            if !report.fonts.missing_ascii.is_empty()
                || !report.fonts.missing_topbar_sample_glyphs.is_empty()
            {
                eprintln!("font coverage gate failed");
                std::process::exit(1);
            }
            let minimum_png_dumps = 13 + EXPECTED_MAJOR_VIEW_COUNT;
            if report.png_dumps.len() < minimum_png_dumps {
                eprintln!(
                    "expected at least {} PNG dump files, got {}",
                    minimum_png_dumps,
                    report.png_dumps.len()
                );
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
