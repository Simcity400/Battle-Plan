use super::*;

fn gradient(width: u32, height: u32) -> Image {
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            rgb.push((x * 3 % 200) as u8);
            rgb.push((y * 5 % 200) as u8);
            rgb.push(((x + y) * 7 % 200) as u8);
        }
    }
    Image::from_rgb(width, height, rgb).unwrap()
}

fn solid(width: u32, height: u32, colour: [u8; 3]) -> Image {
    let rgb = colour
        .iter()
        .cycle()
        .take(width as usize * height as usize * 3)
        .copied()
        .collect();
    Image::from_rgb(width, height, rgb).unwrap()
}

fn offset_rect(image: &Image, rect: Rect, delta: i16) -> Image {
    let mut rgb = image.rgb().to_vec();
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            let base = (y as usize * image.width() as usize + x as usize) * 3;
            for channel in &mut rgb[base..base + 3] {
                *channel = (*channel as i16 + delta).clamp(0, 255) as u8;
            }
        }
    }
    Image::from_rgb(image.width(), image.height(), rgb).unwrap()
}

fn whole(image: &Image) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: image.width(),
        height: image.height(),
    }
}

fn close(value: Option<f64>, expected: f64) -> bool {
    value.is_some_and(|value| (value - expected).abs() < 1e-9)
}

#[test]
fn identical_images_have_no_error_and_perfect_structure() {
    let image = gradient(64, 48);
    let mask = Mask::full(64, 48).unwrap();
    let report = compare(&image, &image, &mask, 4, 4).unwrap();
    assert_eq!(report.overall.pixels, 64 * 48);
    assert_eq!(report.excluded_pixels, 0);
    assert!(close(report.overall.mae, 0.0));
    assert!(close(report.overall.delta_e, 0.0));
    for channel in report.overall.mae_channels {
        assert!(close(channel, 0.0));
    }
    for level in report.overall.ssim_levels {
        assert!(close(level, 1.0), "expected SSIM 1, got {level:?}");
    }
    for region in &report.regions {
        assert!(close(region.metrics.mae, 0.0));
        assert!(close(region.metrics.ssim_levels[0], 1.0));
    }
}

#[test]
fn a_constant_offset_gives_that_offset_as_the_mean_absolute_error() {
    let game = gradient(64, 48);
    let viewer = offset_rect(&game, whole(&game), 10);
    let mask = Mask::full(64, 48).unwrap();
    let report = compare(&game, &viewer, &mask, 2, 2).unwrap();
    assert!(close(report.overall.mae, 10.0));
    for channel in report.overall.mae_channels {
        assert!(close(channel, 10.0));
    }
    assert!(report.overall.delta_e.unwrap() > 0.0);
    for region in &report.regions {
        assert!(close(region.metrics.mae, 10.0));
    }
}

#[test]
fn an_uneven_offset_averages_the_channels() {
    let game = solid(32, 32, [10, 20, 30]);
    let viewer = solid(32, 32, [16, 20, 60]);
    let mask = Mask::full(32, 32).unwrap();
    let report = compare(&game, &viewer, &mask, 1, 1).unwrap();
    assert!(close(report.overall.mae_channels[0], 6.0));
    assert!(close(report.overall.mae_channels[1], 0.0));
    assert!(close(report.overall.mae_channels[2], 30.0));
    assert!(close(report.overall.mae, 12.0));
}

#[test]
fn masking_removes_the_offset_region() {
    let game = gradient(64, 48);
    let wrong = Rect {
        x: 40,
        y: 30,
        width: 20,
        height: 15,
    };
    let viewer = offset_rect(&game, wrong, 20);
    let open = Mask::full(64, 48).unwrap();
    let unmasked = compare(&game, &viewer, &open, 1, 1).unwrap();
    assert!(unmasked.overall.mae.unwrap() > 0.0);
    assert!(unmasked.overall.ssim_levels[0].unwrap() < 1.0);

    let mut masked = Mask::full(64, 48).unwrap();
    masked.exclude_rect(wrong);
    let report = compare(&game, &viewer, &masked, 1, 1).unwrap();
    assert_eq!(report.excluded_pixels, 20 * 15);
    assert_eq!(report.overall.pixels, 64 * 48 - 20 * 15);
    assert!(close(report.overall.mae, 0.0));
    assert!(close(report.overall.delta_e, 0.0));
    assert!(close(report.overall.ssim_levels[0], 1.0));
}

#[test]
fn masking_every_pixel_leaves_no_metrics() {
    let game = gradient(32, 32);
    let viewer = offset_rect(&game, whole(&game), 5);
    let mut mask = Mask::full(32, 32).unwrap();
    mask.exclude_rect(whole(&game));
    let report = compare(&game, &viewer, &mask, 2, 2).unwrap();
    assert_eq!(report.overall.pixels, 0);
    assert_eq!(report.excluded_pixels, 32 * 32);
    assert_eq!(report.overall.mae, None);
    assert_eq!(report.overall.delta_e, None);
    assert_eq!(report.overall.ssim_levels, [None; SSIM_LEVELS]);
    assert!(report.to_text().contains("mae=n/a"));
}

#[test]
fn region_indices_are_row_major_and_tile_the_frame() {
    let game = gradient(96, 48);
    let wrong = Rect {
        x: 24,
        y: 24,
        width: 24,
        height: 24,
    };
    let viewer = offset_rect(&game, wrong, 30);
    let mask = Mask::full(96, 48).unwrap();
    let report = compare(&game, &viewer, &mask, 4, 2).unwrap();
    assert_eq!(report.regions.len(), 8);
    let mut covered = 0u64;
    for (position, region) in report.regions.iter().enumerate() {
        assert_eq!(region.index as usize, position);
        assert_eq!(region.index, region.row * report.columns + region.column);
        assert_eq!(region.row, position as u32 / 4);
        assert_eq!(region.column, position as u32 % 4);
        covered += region.rect.width as u64 * region.rect.height as u64;
    }
    assert_eq!(covered, 96 * 48);
    let target = &report.regions[5];
    assert_eq!((target.row, target.column), (1, 1));
    assert_eq!(target.rect, wrong);
    assert!(close(target.metrics.mae, 30.0));
    for (position, region) in report.regions.iter().enumerate() {
        if position != 5 {
            assert!(close(region.metrics.mae, 0.0), "cell {position} differs");
        }
    }
}

#[test]
fn region_cells_tile_a_size_the_grid_does_not_divide() {
    let image = gradient(37, 23);
    let mask = Mask::full(37, 23).unwrap();
    let report = compare(&image, &image, &mask, 4, 3).unwrap();
    let mut covered = 0u64;
    for region in &report.regions {
        covered += region.rect.width as u64 * region.rect.height as u64;
        assert!(region.rect.right() <= 37 && region.rect.bottom() <= 23);
    }
    assert_eq!(covered, 37 * 23);
}

#[test]
fn small_regions_report_no_ssim_rather_than_failing() {
    let image = gradient(96, 48);
    let mask = Mask::full(96, 48).unwrap();
    let report = compare(&image, &image, &mask, 4, 2).unwrap();
    let cell = &report.regions[0].metrics;
    assert!(close(cell.ssim_levels[0], 1.0));
    assert!(close(cell.ssim_levels[1], 1.0));
    assert_eq!(cell.ssim_levels[2], None);
}

#[test]
fn the_colour_distance_sees_a_hue_change_the_luminance_ssim_misses() {
    let purple = [150u8, 50, 200];
    let green = [40u8, 140, 40];
    assert!((luminance(purple) - luminance(green)).abs() < 2.0);
    let mask = Mask::full(32, 32).unwrap();
    let report = compare(&solid(32, 32, purple), &solid(32, 32, green), &mask, 1, 1).unwrap();
    assert!(report.overall.ssim_levels[0].unwrap() > 0.999);
    assert!(report.overall.delta_e.unwrap() > 20.0);
    assert!(close(report.overall.mae, 120.0));
}

#[test]
fn a_size_mismatch_is_an_error() {
    let game = gradient(32, 32);
    let viewer = gradient(32, 16);
    let mask = Mask::full(32, 32).unwrap();
    let error = compare(&game, &viewer, &mask, 1, 1).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("image size mismatch"));
    assert!(diff_image(&game, &viewer, &mask).is_err());
}

#[test]
fn a_mask_size_mismatch_is_an_error() {
    let image = gradient(32, 32);
    let mask = Mask::full(16, 16).unwrap();
    let error = compare(&image, &image, &mask, 1, 1).unwrap_err();
    assert!(error.to_string().contains("mask size mismatch"));
}

#[test]
fn a_region_grid_larger_than_the_image_is_an_error() {
    let image = gradient(16, 16);
    let mask = Mask::full(16, 16).unwrap();
    assert!(compare(&image, &image, &mask, 0, 4).is_err());
    assert!(compare(&image, &image, &mask, 4, 0).is_err());
    assert!(compare(&image, &image, &mask, 17, 4).is_err());
    assert!(compare(&image, &image, &mask, 4, 17).is_err());
}

#[test]
fn a_mask_image_excludes_black_pixels_only() {
    let mut rgb = vec![255u8; 4 * 2 * 3];
    rgb[0..3].copy_from_slice(&[0, 0, 0]);
    rgb[3..6].copy_from_slice(&[0, 0, 1]);
    let mask = Mask::from_image(&Image::from_rgb(4, 2, rgb).unwrap());
    assert!(!mask.includes(0, 0));
    assert!(mask.includes(1, 0));
    assert!(mask.includes(3, 1));
    assert_eq!(mask.included_pixels(), 7);
}

#[test]
fn excluded_rectangles_clip_to_the_frame() {
    let mut mask = Mask::full(8, 4).unwrap();
    mask.exclude_rect(Rect {
        x: 6,
        y: 2,
        width: 100,
        height: 100,
    });
    assert_eq!(mask.included_pixels(), 8 * 4 - 4);
    assert!(!mask.includes(7, 3));
    assert!(mask.includes(5, 3));
    mask.exclude_rect(Rect {
        x: 20,
        y: 20,
        width: 4,
        height: 4,
    });
    assert_eq!(mask.included_pixels(), 8 * 4 - 4);
    assert!(!mask.includes(9, 9));
}

#[test]
fn the_difference_image_amplifies_and_greys_masked_pixels() {
    let game = solid(4, 2, [10, 20, 30]);
    let viewer = solid(4, 2, [20, 20, 100]);
    let mut mask = Mask::full(4, 2).unwrap();
    mask.exclude_rect(Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    let diff = diff_image(&game, &viewer, &mask).unwrap();
    assert_eq!(diff.pixel(0, 0), Some(DIFF_MASKED));
    assert_eq!(diff.pixel(1, 0), Some([40, 0, 255]));
    assert_eq!(diff.pixel(3, 1), Some([40, 0, 255]));
}

#[test]
fn rectangles_parse_from_the_command_line_form() {
    assert_eq!(
        Rect::parse("1,2,3,4").unwrap(),
        Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4
        }
    );
    assert_eq!(Rect::parse(" 1 , 2 , 3 , 4 ").unwrap().width, 3);
    for bad in [
        "1,2,3",
        "1,2,3,4,5",
        "1,2,0,4",
        "1,2,3,0",
        "-1,2,3,4",
        "a,b,c,d",
        "",
    ] {
        assert!(Rect::parse(bad).is_err(), "`{bad}` should be rejected");
    }
}

#[test]
fn images_reject_a_buffer_that_does_not_match_the_dimensions() {
    assert!(Image::from_rgb(2, 2, vec![0; 12]).is_ok());
    assert!(Image::from_rgb(2, 2, vec![0; 11]).is_err());
    assert!(Image::from_rgb(0, 2, Vec::new()).is_err());
    assert!(Mask::full(2, 0).is_err());
}

fn encode(width: u32, height: u32, colour: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(colour);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
fn rgb_and_rgba_pngs_decode_to_the_same_pixels() {
    let rgb = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let rgba = vec![1u8, 2, 3, 200, 4, 5, 6, 201, 7, 8, 9, 202, 10, 11, 12, 203];
    let from_rgb = decode_png(&encode(2, 2, png::ColorType::Rgb, &rgb)).unwrap();
    let from_rgba = decode_png(&encode(2, 2, png::ColorType::Rgba, &rgba)).unwrap();
    assert_eq!(from_rgb.rgb(), rgb.as_slice());
    assert_eq!(from_rgb, from_rgba);
    assert_eq!(from_rgb.pixel(1, 1), Some([10, 11, 12]));
    assert_eq!(from_rgb.pixel(2, 0), None);
}

#[test]
fn a_greyscale_png_and_a_non_png_are_named_errors() {
    let grey = encode(2, 2, png::ColorType::Grayscale, &[1, 2, 3, 4]);
    let error = decode_png(&grey).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("RGB or RGBA"), "{error}");
    assert!(decode_png(b"not a png at all").is_err());
}

#[test]
fn upscaling_two_by_two_to_four_by_four_aligns_pixel_centres() {
    let image = Image::from_rgb(2, 2, vec![0, 0, 0, 100, 0, 0, 200, 0, 0, 40, 0, 0]).unwrap();
    let scaled = upscale(&image, 2.0).unwrap();
    assert_eq!((scaled.width(), scaled.height()), (4, 4));
    let row: Vec<u8> = (0..4).map(|x| scaled.pixel(x, 0).unwrap()[0]).collect();
    assert_eq!(row, vec![0, 25, 75, 100]);
    let row: Vec<u8> = (0..4).map(|x| scaled.pixel(x, 3).unwrap()[0]).collect();
    assert_eq!(row, vec![200, 160, 80, 40]);
    assert_eq!(scaled.pixel(0, 1).unwrap()[0], 50);
    assert_eq!(scaled.pixel(3, 2).unwrap()[0], 55);
}

#[test]
fn upscaling_by_one_and_a_quarter_reaches_the_screenshot_size() {
    let scaled = upscale(&gradient(2048, 8), 1.25).unwrap();
    assert_eq!(scaled.width(), 2560);
    assert_eq!(scaled.height(), 10);
    assert!(upscale(&gradient(4, 4), 0.0).is_err());
    assert!(upscale(&gradient(4, 4), f64::NAN).is_err());
    assert_eq!(upscale(&gradient(4, 4), 1.0).unwrap(), gradient(4, 4));
}
