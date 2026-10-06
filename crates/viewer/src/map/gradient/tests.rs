use super::*;

/// Two regions split down the middle of a 64 x 32 map, both reaching the top and bottom edges.
fn split_map(ids: &mut Vec<u32>, width: u32, height: u32, seam: u32) {
    ids.clear();
    for _ in 0..height {
        for x in 0..width {
            ids.push(if x < seam { 1 } else { 2 });
        }
    }
}

fn colors(region: u32) -> [u8; 3] {
    match region {
        1 => [10, 20, 30],
        2 => [40, 50, 60],
        _ => [0, 0, 0],
    }
}

#[test]
fn hsv_sectors_outside_zero_to_four_take_the_last_case() {
    assert_eq!(hsv_to_rgb([0.5, 0.5, 1.0]), [0.5, 1.0, 1.0]);
    // Hue -0.25 is sector -2: the native chain's last case, not the first.
    assert_eq!(hsv_to_rgb([-0.25, 0.5, 1.0]), [1.0, 0.5, 0.75]);
}

#[test]
fn the_field_peaks_on_the_boundary_and_dies_inside_the_region() {
    // Section 1.4 worked through by hand for a state layer, T = 5, C = 2.
    assert_eq!(field_alpha(0.0, 5.0, 2.0), 247, "255 - 0 - 8");
    assert_eq!(field_alpha(1.0, 5.0, 2.0), 246, "255 - 1 - 8");
    assert_eq!(
        field_alpha(2.0, 5.0, 2.0),
        245,
        "255 - 2 - 8, the last centre pixel"
    );
    // d = 3: 255 - floor(1 * 255 / 5) - 2 - 8 = 255 - 51 - 10.
    assert_eq!(field_alpha(3.0, 5.0, 2.0), 194);
    assert_eq!(field_alpha(4.0, 5.0, 2.0), 143);
    // The field reaches zero at C + T * 247 / 255 = 6.84 pixels.
    assert_eq!(field_alpha(6.8, 5.0, 2.0), 1);
    assert_eq!(field_alpha(6.9, 5.0, 2.0), 0);
    assert_eq!(field_alpha(30.0, 5.0, 2.0), 0);
    // A thick country field, T = 25, reaches zero at 26.2 pixels.
    assert_eq!(field_alpha(0.0, 25.0, 2.0), 247);
    assert_eq!(field_alpha(3.0, 25.0, 2.0), 235, "255 - 10 - 2 - 8");
    assert_eq!(field_alpha(26.0, 25.0, 2.0), 1);
    assert_eq!(field_alpha(26.2, 25.0, 2.0), 0);
    // Degenerate inputs are answered, not panicked on.
    assert_eq!(field_alpha(1.0, 0.0, 2.0), 0);
    assert_eq!(field_alpha(f32::NAN, 5.0, 2.0), 0);
}

#[test]
fn country_thickness_runs_from_five_to_twenty_five_pixels() {
    assert_eq!(country_thickness(0, &FieldDefines::TEST), 5.0);
    assert_eq!(country_thickness(299, &FieldDefines::TEST), 5.0);
    assert_eq!(country_thickness(300, &FieldDefines::TEST), 5.0);
    assert_eq!(country_thickness(9000, &FieldDefines::TEST), 25.0);
    assert_eq!(country_thickness(9001, &FieldDefines::TEST), 25.0);
    assert_eq!(country_thickness(u64::MAX, &FieldDefines::TEST), 25.0);
    // Halfway up the area range is halfway up the thickness range.
    assert!((country_thickness((300 + 9000) / 2, &FieldDefines::TEST) - 15.0).abs() < 0.001);
    // The four rows of the module header's table that the capture constrains best.
    for (area, thickness) in [(460, 5.4), (4616, 14.9), (7476, 21.5), (1_217_260, 25.0)] {
        let fitted = country_thickness(area, &FieldDefines::TEST);
        assert!((fitted - thickness).abs() < 0.05, "{area}: {fitted}");
    }
    assert_eq!(
        LayerKind::Fixed(5.0).thickness(u64::MAX, &FieldDefines::TEST),
        5.0
    );
    assert_eq!(
        LayerKind::Fixed(150.0).thickness(0, &FieldDefines::TEST),
        150.0
    );
    assert_eq!(LayerKind::Country.thickness(0, &FieldDefines::TEST), 5.0);
}

#[test]
fn the_optimization_range_never_truncates_a_thicker_field() {
    // A country field bottoms out inside the range, so the range is a no-op on it.
    assert_eq!(field_reach(5.0, &FieldDefines::TEST), 30.0);
    assert_eq!(field_reach(25.0, &FieldDefines::TEST), 30.0);
    assert_eq!(field_alpha(30.0, 25.0, 2.0), 0);
    // The strategic-region field does not, and reading the range as a cap would cut it at 30
    // map pixels where the game's own texture carries it past 60.
    assert_eq!(field_reach(150.0, &FieldDefines::TEST), 152.0);
    assert_eq!(field_alpha(60.0, 150.0, 2.0), 147);
}

#[test]
fn the_textures_have_the_engines_shape_and_guard_rows() {
    let mut ids = Vec::new();
    split_map(&mut ids, 64, 32, 32);
    let map = ProvinceMap {
        width: 64,
        height: 32,
        ids: &ids,
    };
    let region_of = [0, 1, 2];
    let country = Layer::new(LayerKind::Country, &region_of, &colors, &FieldDefines::TEST);
    let state = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &country, &state).unwrap();
    assert_eq!(textures.width, 32);
    assert_eq!(textures.height, 2 * (16 + 1));
    assert_eq!(textures.layer_rows(), 16);
    assert_eq!(textures.channel1.len(), 32 * 34);
    assert_eq!(textures.channel2.len(), 32 * 34);
    // Row H and the last row are the guard rows the shader's HalfPix avoids.
    for column in 0..32 {
        assert_eq!(textures.channel1[16 * 32 + column], [0; 4], "layer 1 guard");
        assert_eq!(textures.channel2[16 * 32 + column], [0; 4], "layer 1 guard");
        assert_eq!(textures.channel1[33 * 32 + column], [0; 4], "layer 2 guard");
        assert_eq!(textures.channel2[33 * 32 + column], [0; 4], "layer 2 guard");
    }
    assert_eq!(textures.layer_origin(0), 0);
    assert_eq!(textures.layer_origin(1), 17);
    assert_eq!(textures.channel1_texel(2, 0, 0), None);
    assert_eq!(textures.channel1_texel(0, 32, 0), None);
    assert_eq!(textures.channel1_texel(0, 0, 16), None);
}

#[test]
fn the_field_rows_run_top_down_like_the_shader_coordinate() {
    // Only the top two map rows are in a region, so the field may only appear in the plane's
    // first row: the shader reads the field at `uv2`, which is 0 at the map's north edge.
    let width = 64;
    let height = 32;
    let mut ids = Vec::new();
    for y in 0..height {
        for _ in 0..width {
            ids.push(if y < 2 { 1 } else { 0 });
        }
    }
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    let rows = textures.layer_rows();
    for x in 0..textures.width {
        assert_eq!(
            textures.channel1_texel(0, x, 0).unwrap(),
            [10, 20, 30, 246],
            "map row 1 is the boundary and row 0 is one pixel inland"
        );
        assert_eq!(textures.channel2_texel(0, x, 0).unwrap(), [0, 255, 0, 0]);
        assert_eq!(
            textures.channel1_texel(0, x, 1).unwrap(),
            [5, 10, 15, NO_REGION_ALPHA]
        );
        assert_eq!(textures.channel2_texel(0, x, 1).unwrap(), [0, 127, 0, 0]);
        for y in 2..rows {
            assert_eq!(
                textures.channel1_texel(0, x, y).unwrap(),
                [0, 0, 0, NO_REGION_ALPHA]
            );
            assert_eq!(textures.channel2_texel(0, x, y).unwrap(), [0; 4]);
        }
    }
}

#[test]
fn a_plane_texel_is_the_box_average_of_its_two_by_two_block() {
    let width = 64;
    let height = 32;
    let mut ids = Vec::new();
    split_map(&mut ids, width, height, 32);
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1, 2];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    // Plane column 15 covers map columns 30 and 31, plane row 8 covers map rows 15 and 14.
    // Column 31 is the boundary itself and column 30 is one pixel inside it.
    let texel = textures.channel1_texel(0, 15, 8).unwrap();
    let expected =
        (u32::from(field_alpha(1.0, 5.0, 2.0)) * 2 + u32::from(field_alpha(0.0, 5.0, 2.0)) * 2) / 4;
    assert_eq!(texel, [10, 20, 30, expected as u8]);
    assert_eq!(
        texel[3], 246,
        "vertical averages 246 and 247, then floor their mean"
    );
    // Plane column 16 straddles nothing: map columns 32 and 33 are both region 2, so only the
    // alpha differs across the block and the colour survives the average intact.
    let across = textures.channel1_texel(0, 16, 8).unwrap();
    assert_eq!(across, [40, 50, 60, 246]);
    // Deep interior: map columns 16 and 17 are 14 pixels from every boundary, past the reach of
    // a five pixel field.
    assert_eq!(textures.channel1_texel(0, 8, 8).unwrap(), [10, 20, 30, 0]);
}

#[test]
fn a_blended_block_averages_both_regions() {
    // One region occupying a single column next to another makes a plane texel that straddles
    // the two, so the box average of the colours is visible.
    let width = 64;
    let height = 32;
    let mut ids = Vec::new();
    for _ in 0..height {
        for x in 0..width {
            ids.push(if x < 31 { 1 } else { 2 });
        }
    }
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1, 2];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    // Plane column 15 covers map columns 30 (region 1, on the boundary) and 31 (region 2, on
    // the boundary), both at alpha 247.
    let texel = textures.channel1_texel(0, 15, 8).unwrap();
    assert_eq!(texel, [25, 35, 45, 247]);
}

#[test]
fn regions_are_adjacent_across_the_maps_x_edge() {
    // Region 1 fills the left half and region 2 the right half, so with the world wrapping the
    // column at x = 0 borders region 2 at x = 63 and carries the full field. Without the wrap
    // its nearest boundary would be 31 pixels away and the field would be zero.
    let width = 64;
    let height = 32;
    let mut ids = Vec::new();
    split_map(&mut ids, width, height, 32);
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1, 2];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    // Plane column 0 covers map columns 0 and 1: on the seam and one pixel in.
    let seam = textures.channel1_texel(0, 0, 8).unwrap();
    assert_eq!(seam[3], 246, "vertical then horizontal truncation");
    let other_side = textures.channel1_texel(0, 31, 8).unwrap();
    assert_eq!(other_side, [40, 50, 60, 246], "map columns 62 and 63");
    // Two columns in the field has already dropped by the slope.
    let inside =
        2 * u32::from(field_alpha(2.0, 5.0, 2.0)) + 2 * u32::from(field_alpha(3.0, 5.0, 2.0));
    assert_eq!(
        textures.channel1_texel(0, 1, 8).unwrap()[3],
        (inside / 4) as u8
    );
    assert_eq!(textures.channel1_texel(0, 1, 8).unwrap()[3], 219);
}

#[test]
fn a_region_opacity_lands_in_channel_two() {
    let width = 8;
    let height = 4;
    let ids = vec![1_u32; (width * height) as usize];
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 7];
    let mut layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    layer.region_opacity = 241;
    let textures = build(&map, &layer, &layer).unwrap();
    let rows = textures.layer_rows();
    for y in 0..rows {
        for x in 0..textures.width {
            assert_eq!(textures.channel2_texel(0, x, y).unwrap(), [0, 241, 0, 0]);
        }
    }
    // Region key 7 is not one the test's colour table knows, and it still resolves through the
    // caller's closure rather than being invented here. One region covering the whole map has
    // no boundary at all - the map wraps in x and the poles are not boundaries - so its field
    // is zero everywhere and its interior is all the layer is.
    assert_eq!(textures.channel1_texel(0, 0, 0).unwrap(), [0, 0, 0, 0]);
}

#[test]
fn a_region_interior_keeps_its_colour_and_opacity_with_no_field_left() {
    // One region far wider than `30.0`, so its middle is out of the boundary
    // search's reach entirely and takes the "no boundary in range" early-out.
    let width = 256;
    let height = 128;
    let mut ids = Vec::new();
    for _ in 0..height {
        for x in 0..width {
            ids.push(if (32..224).contains(&x) { 1 } else { 2 });
        }
    }
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1, 2];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    let middle = textures
        .channel1_texel(0, textures.width / 2, textures.layer_rows() / 2)
        .unwrap();
    assert_eq!(
        middle[..3],
        [10, 20, 30],
        "the interior keeps the region colour"
    );
    assert_eq!(middle[3], 0, "and has no distance field left");
    assert_eq!(
        textures
            .channel2_texel(0, textures.width / 2, textures.layer_rows() / 2)
            .unwrap(),
        [0, 255, 0, 0],
        "the interior keeps its opacity, which is what `TranspA` reads"
    );
}

#[test]
fn sea_provinces_stay_out_of_the_layer() {
    // Region 0 is the "no region" key: those pixels are zero in both textures and do not make
    // their neighbours' field disappear.
    let width = 16;
    let height = 4;
    let mut ids = Vec::new();
    for _ in 0..height {
        for x in 0..width {
            ids.push(if x < 8 { 1_u32 } else { 5 });
        }
    }
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let region_of = [0, 1, 0, 0, 0, 0];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let textures = build(&map, &layer, &layer).unwrap();
    assert_eq!(textures.channel1_texel(0, 0, 0).unwrap(), [10, 20, 30, 246]);
    assert_eq!(
        textures.channel1_texel(0, 7, 0).unwrap(),
        [0, 0, 0, NO_REGION_ALPHA],
        "a pixel in no region is the alpha the shader draws nothing at"
    );
    assert_eq!(textures.channel2_texel(0, 7, 0).unwrap(), [0; 4]);
}

#[test]
fn bad_input_is_a_named_error_rather_than_a_panic() {
    let ids = vec![1_u32; 8];
    let region_of = [0, 1];
    let layer = Layer::new(
        LayerKind::Fixed(5.0),
        &region_of,
        &colors,
        &FieldDefines::TEST,
    );
    let short = ProvinceMap {
        width: 4,
        height: 4,
        ids: &ids,
    };
    assert!(
        build(&short, &layer, &layer)
            .unwrap_err()
            .to_string()
            .contains("8 ids for 4 x 4")
    );
    let odd = ProvinceMap {
        width: 8,
        height: 1,
        ids: &ids,
    };
    assert!(
        build(&odd, &layer, &layer)
            .unwrap_err()
            .to_string()
            .contains("divisible by")
    );
    let empty = ProvinceMap {
        width: 0,
        height: 2,
        ids: &[],
    };
    assert!(build(&empty, &layer, &layer).is_err());
    let unknown = vec![9_u32; 8];
    let outside = ProvinceMap {
        width: 4,
        height: 2,
        ids: &unknown,
    };
    assert!(
        build(&outside, &layer, &layer)
            .unwrap_err()
            .to_string()
            .contains("province 9 is outside")
    );
}

#[test]
fn the_two_layers_are_built_independently() {
    // The same map with different region tables: layer 1 sees one region, layer 2 sees two.
    let width = 64;
    let height = 32;
    let mut ids = Vec::new();
    split_map(&mut ids, width, height, 32);
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let merged = [0, 1, 1];
    let split = [0, 1, 2];
    let country = Layer::new(LayerKind::Country, &merged, &colors, &FieldDefines::TEST);
    let state = Layer::new(LayerKind::Fixed(5.0), &split, &colors, &FieldDefines::TEST);
    let textures = build(&map, &country, &state).unwrap();
    // 64 * 32 = 2048 map pixels is a fifth of the way up the 300 to 9000 area range, so a
    // country that size takes T = 9.02 and a field that reaches 10.7 pixels.
    assert!((country_thickness(2048, &FieldDefines::TEST) - 9.018).abs() < 0.001);
    // Layer 1 is one region covering the map, so it has no boundary anywhere: the map wraps in
    // x and the poles are not boundaries. Every texel of it is region interior.
    let rows = textures.layer_rows();
    for y in [0, 8, rows - 1] {
        assert_eq!(textures.channel1_texel(0, 15, y).unwrap(), [10, 20, 30, 0]);
        assert_eq!(textures.channel2_texel(0, 15, y).unwrap(), [0, 255, 0, 0]);
    }
    // Layer 2 still has the seam down the middle, and its own thickness rule.
    assert_eq!(textures.channel1_texel(1, 15, 8).unwrap()[3], 246);
    assert_eq!(textures.channel1_texel(1, 8, 8).unwrap(), [10, 20, 30, 0]);
}
