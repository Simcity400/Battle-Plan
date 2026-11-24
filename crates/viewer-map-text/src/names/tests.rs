use super::*;

struct TestFont {
    advance: f32,
    line_height: f32,
    kern: Option<(char, char, f32)>,
    colors: HashMap<char, [f32; 4]>,
}

impl TestFont {
    fn new() -> Self {
        TestFont {
            advance: 6.0,
            line_height: 10.0,
            kern: None,
            colors: HashMap::new(),
        }
    }

    fn with_kerning(a: char, b: char, amount: f32) -> Self {
        TestFont {
            kern: Some((a, b, amount)),
            ..TestFont::new()
        }
    }

    fn with_color(mut self, code: char, color: [f32; 4]) -> Self {
        self.colors.insert(code, color);
        self
    }

    fn known(ch: char) -> bool {
        ch == ' ' || ch.is_ascii_alphanumeric()
    }
}

impl GlyphMetrics for TestFont {
    fn advance(&self, ch: char) -> Option<f32> {
        TestFont::known(ch).then_some(self.advance)
    }

    fn line_height(&self) -> f32 {
        self.line_height
    }

    fn base(&self) -> f32 {
        self.line_height * 0.8
    }

    fn glyph(&self, ch: char) -> Option<GlyphBox> {
        if !TestFont::known(ch) {
            return None;
        }
        if ch == ' ' {
            return Some(GlyphBox {
                width: 3.0,
                height: 2.0,
                xoffset: 0.0,
                yoffset: 0.0,
                xadvance: self.advance,
                uv: [0.0; 4],
            });
        }
        Some(GlyphBox {
            width: self.advance - 1.0,
            height: 7.0,
            xoffset: 1.0,
            yoffset: 2.0,
            xadvance: self.advance,
            uv: [0.1, 0.2, 0.3, 0.4],
        })
    }

    fn kerning(&self, a: char, b: char) -> f32 {
        match self.kern {
            Some((ka, kb, amount)) if ka == a && kb == b => amount,
            _ => 0.0,
        }
    }

    fn text_color(&self, code: char) -> Option<[f32; 4]> {
        self.colors.get(&code).copied()
    }
}

struct Canvas {
    width: u32,
    height: u32,
    ids: Vec<u32>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Canvas {
            width,
            height,
            ids: vec![0; (width * height) as usize],
        }
    }

    fn rect(&mut self, id: u32, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.ids[(y * self.width + x) as usize] = id;
            }
        }
    }

    fn map(&self) -> ProvinceMap<'_> {
        ProvinceMap {
            width: self.width,
            height: self.height,
            ids: &self.ids,
        }
    }
}

fn dry() -> HashSet<u32> {
    HashSet::new()
}

fn place<'a>(
    text: &'a str,
    canvas: &'a Canvas,
    area: &'a [u32],
    font: &TestFont,
) -> Option<PlacedName> {
    place_name(
        &NameInput {
            text,
            provinces: canvas.map(),
            area,
            bridged: &dry(),
            bounds: None,
        },
        font,
    )
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn horizontal_rectangle_gets_a_horizontal_width_limited_baseline() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 30, 179, 69);
    let font = TestFont::new();
    let placed = place("HorizontalBand", &canvas, &[1], &font).expect("placed");

    assert_eq!(placed.baseline, ([20.0, 53.0], [179.0, 53.0]));
    let expected = FILL * 159.0 / 84.0 * FINAL_SCALE;
    assert!(close(placed.scale, expected, 1e-4), "{placed:?}");
    assert!(close(placed.font_size, expected * FONT_SIZE_CONV, 1e-3));
    assert_eq!(placed.spaces_inserted, 0);
    assert_eq!(placed.glyphs.len(), 14);
    let centres: Vec<_> = placed
        .glyphs
        .iter()
        .map(|g| midpoint(g.corners[0], g.corners[2]))
        .collect();
    assert!(centres[0][1] > centres[centres.len() / 2][1]);
    assert!(placed.aabb[0] < placed.aabb[2] && placed.aabb[1] < placed.aabb[3]);
}

#[test]
fn tall_rectangle_takes_the_vertical_axis() {
    let mut canvas = Canvas::new(200, 200);
    canvas.rect(1, 80, 10, 119, 189);
    let font = TestFont::new();
    let placed = place("TallState", &canvas, &[1], &font).expect("placed");

    let (start, end) = placed.baseline;
    assert!(close(start[0], end[0], 1e-3), "{placed:?}");
    assert!((end[1] - start[1]).abs() > (end[0] - start[0]).abs());
    assert_eq!(placed.baseline, ([102.0, 189.0], [102.0, 10.0]));
}

#[test]
fn the_axis_bias_keeps_a_slightly_taller_area_horizontal() {
    let mut canvas = Canvas::new(200, 200);
    canvas.rect(1, 50, 50, 149, 154);
    let font = TestFont::new();
    let placed = place("Squarish", &canvas, &[1], &font).expect("placed");

    let (start, end) = placed.baseline;
    assert!(
        close(start[1], end[1], 1e-3) && (end[0] - start[0]).abs() > 90.0,
        "{placed:?}"
    );
    let length_h_sq: f32 = 99.0 * 99.0;
    let length_v_sq: f32 = 104.0 * 104.0;
    assert!(length_h_sq * AXIS_BIAS > length_v_sq);
}

#[test]
fn an_l_shape_clips_its_endpoints_into_the_area() {
    let mut canvas = Canvas::new(200, 200);
    canvas.rect(1, 20, 100, 179, 119);
    canvas.rect(2, 20, 20, 49, 99);
    let font = TestFont::new();
    let placed = place("Elbow", &canvas, &[1, 2], &font).expect("placed");

    let map = canvas.map();
    for point in [placed.baseline.0, placed.baseline.1] {
        let id = map
            .id_at(point[0].round() as i32, point[1].round() as i32)
            .expect("inside the image");
        assert!(id == 1 || id == 2, "endpoint {point:?} left the area");
    }
    let span = (placed.baseline.1[0] - placed.baseline.0[0]).abs();
    assert!(span < 159.0, "{placed:?}");
}

#[test]
fn a_thin_area_inserts_spaces_until_the_width_condition_is_met() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 50, 40, 149, 59);
    let font = TestFont::new();
    let placed = place("ABCDE", &canvas, &[1], &font).expect("placed");

    let length = 99.0f32;
    let width_scale = FILL * length / 30.0;
    let height_scale = HALF * (2000.0 / length) / 10.0;
    assert!(
        height_scale < width_scale,
        "the case must be height limited"
    );
    assert!(close(placed.scale, height_scale * FINAL_SCALE, 1e-4));

    let target = FILL * length / height_scale;
    let width_at = |spaces: usize| (5.0 + 4.0 * spaces as f32) * 6.0;
    assert!(width_at(2) < target && target <= width_at(3));
    assert_eq!(placed.spaces_inserted, 2);
    assert_eq!(placed.glyphs.len(), 5);
    assert!(placed.glyphs.iter().all(|g| g.ch != ' '));
}

#[test]
fn a_gap_along_the_fit_cuts_the_baseline_to_the_longer_side() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 40, 79, 59);
    canvas.rect(9, 80, 40, 119, 59);
    canvas.rect(1, 120, 40, 179, 59);
    let font = TestFont::new();
    let placed = place("Split", &canvas, &[1], &font).expect("placed");

    let (start, end) = placed.baseline;
    assert_eq!(end[0] - start[0], 59.0, "{placed:?}");
    assert!(start[0] >= 20.0 && end[0] <= 179.0, "{placed:?}");

    let bridged = HashSet::from([9]);
    let spanned = place_name(
        &NameInput {
            text: "Split",
            provinces: canvas.map(),
            area: &[1],
            bridged: &bridged,
            bounds: None,
        },
        &font,
    )
    .expect("placed");
    let (start, end) = spanned.baseline;
    assert_eq!(end[0] - start[0], 159.0, "{spanned:?}");
}

#[test]
fn the_offsets_are_perpendicular_and_find_the_widest_line() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 44, 39, 45);
    canvas.rect(1, 20, 41, 179, 42);
    let font = TestFont::new();
    let placed = place("Wedge", &canvas, &[1], &font).expect("placed");

    let (start, end) = placed.baseline;
    assert!(end[0] - start[0] > 39.0 - 20.0, "{placed:?}");
    assert!(start[1] <= 42.0 && end[1] <= 42.0, "{placed:?}");
}

#[test]
fn water_only_candidates_survive_the_native_land_trims() {
    let mut canvas = Canvas::new(20, 10);
    canvas.rect(1, 5, 5, 14, 8);
    let area = HashSet::from([1]);
    assert_eq!(
        best_run(
            &canvas.map(),
            &area,
            &HashSet::from([0]),
            [4.0, 8.0],
            [15.0, 8.0],
            None
        ),
        Some((12, [4.0, 8.0], [15.0, 8.0]))
    );
    assert!(best_run(&canvas.map(), &area, &dry(), [4.0, 8.0], [15.0, 8.0], None).is_none());
}

#[test]
fn a_very_thin_area_stops_at_the_nineteen_space_cap() {
    let mut canvas = Canvas::new(400, 100);
    canvas.rect(1, 20, 45, 379, 50);
    let font = TestFont::new();
    let placed = place("ABC", &canvas, &[1], &font).expect("placed");

    let length = 359.0f32;
    let height_scale = HALF * (2160.0 / length) / 10.0;
    let target = FILL * length / height_scale;
    assert!(target > (3.0 + 2.0 * MAX_SPACES as f32) * 6.0);
    assert_eq!(placed.spaces_inserted, MAX_SPACES);
}

#[test]
fn an_area_split_at_both_edges_wraps_across_the_date_line() {
    let mut canvas = Canvas::new(100, 50);
    canvas.rect(1, 0, 20, 9, 29);
    canvas.rect(2, 90, 20, 99, 29);
    let font = TestFont::new();
    let placed = place("Wrapped", &canvas, &[1, 2], &font).expect("placed");

    assert_eq!(placed.baseline, ([90.0, 25.0], [109.0, 25.0]));
    let centre = (placed.baseline.0[0] + placed.baseline.1[0]) * 0.5;
    assert!(centre > 89.0, "{placed:?}");
    let map = canvas.map();
    for x in 90..=109 {
        let id = map.id_at(x, 24).expect("inside the image");
        assert!(id == 1 || id == 2);
    }
}

#[test]
fn kerning_affects_fit_width_while_quads_keep_their_metric_dimensions() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 30, 179, 69);
    let font = TestFont::with_kerning('A', 'B', -2.0);
    let placed = place("abcdefghijklmn", &canvas, &[1], &font).expect("placed");

    let expected_scale = FILL * 159.0 / 82.0 * FINAL_SCALE;
    assert!(close(placed.scale, expected_scale, 1e-4), "{placed:?}");
    assert_eq!(placed.glyphs.len(), 14);
    assert_eq!(placed.glyphs[0].ch, 'A');
    assert_eq!(placed.glyphs[1].ch, 'B');

    let quad = placed.glyphs[2];
    assert!(close(
        distance(quad.corners[0], quad.corners[1]),
        5.0 * placed.scale,
        1e-3
    ));
    assert_eq!(quad.uv, [0.1, 0.2, 0.3, 0.4]);
}

#[test]
fn degenerate_areas_return_none_instead_of_panicking() {
    let font = TestFont::new();

    let mut single = Canvas::new(32, 32);
    single.rect(1, 10, 10, 10, 10);
    assert!(place("Dot", &single, &[1], &font).is_none());

    let mut column = Canvas::new(32, 32);
    column.rect(1, 10, 4, 10, 24);
    assert!(place("Column", &column, &[1], &font).is_none());

    let mut row = Canvas::new(32, 32);
    row.rect(1, 4, 10, 24, 10);
    assert!(place("Row", &row, &[1], &font).is_none());

    let mut plain = Canvas::new(64, 64);
    plain.rect(1, 8, 8, 40, 40);
    assert!(place("Nothing", &plain, &[], &font).is_none());
    assert!(place("Missing", &plain, &[77], &font).is_none());

    assert!(place("!!!", &plain, &[1], &font).is_none());
    assert!(place("", &plain, &[1], &font).is_none());

    let broken = ProvinceMap {
        width: 8,
        height: 8,
        ids: &[0u32; 7],
    };
    assert!(
        place_name(
            &NameInput {
                text: "Broken",
                provinces: broken,
                area: &[1],
                bridged: &dry(),
                bounds: None,
            },
            &font
        )
        .is_none()
    );
    let empty = ProvinceMap {
        width: 0,
        height: 0,
        ids: &[],
    };
    assert!(
        place_name(
            &NameInput {
                text: "Empty",
                provinces: empty,
                area: &[1],
                bridged: &dry(),
                bounds: None,
            },
            &font
        )
        .is_none()
    );
}

#[test]
fn precomputed_bounds_place_the_same_name() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 30, 179, 69);
    let font = TestFont::new();
    let map = canvas.map();
    let bounds = scan_province_bounds(&map);
    assert_eq!(
        bounds.get(&1),
        Some(&ProvinceBounds {
            min_x: 20,
            min_y: 30,
            max_x: 179,
            max_y: 69
        })
    );
    let with = place_name(
        &NameInput {
            text: "HorizontalBand",
            provinces: map,
            area: &[1],
            bridged: &dry(),
            bounds: Some(&bounds),
        },
        &font,
    );
    let without = place("HorizontalBand", &canvas, &[1], &font);
    assert_eq!(with, without);
}

#[test]
fn upper_casing_happens_here_not_in_the_caller() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 30, 179, 69);
    let font = TestFont::new();
    let placed = place("german reich", &canvas, &[1], &font).expect("placed");
    let drawn: String = placed.glyphs.iter().map(|g| g.ch).collect();
    assert_eq!(drawn, "GERMANREICH");
}

#[test]
fn the_currency_control_counts_in_the_fit_but_draws_nothing() {
    let mut canvas = Canvas::new(200, 120);
    canvas.rect(1, 20, 0, 179, 119);
    let font = TestFont::new();
    let placed = place("A\u{14}B", &canvas, &[1], &font).expect("placed");
    assert_eq!(placed.spaces_inserted, 0);
    let length = distance(placed.baseline.0, placed.baseline.1);
    let expected = FILL * length / 28.0 * FINAL_SCALE;
    assert!(close(placed.scale, expected, 1e-4), "{placed:?}");
    let drawn: String = placed.glyphs.iter().map(|g| g.ch).collect();
    assert_eq!(drawn, "AB");
}

#[test]
fn wide_raster_segment_and_degenerate_candidates_stay_raster_bounded() {
    let width = 65_537u32;
    let height = 4u32;
    let mut canvas = Canvas::new(width, height);
    canvas.rect(1, 0, 0, width - 1, height - 1);
    let map = canvas.map();
    let area = HashSet::from([1]);
    let bridged = dry();
    let (score, start, end) = best_run(
        &map,
        &area,
        &bridged,
        [0.0, 1.0],
        [(width - 1) as f32, 1.0],
        None,
    )
    .expect("wide raster horizontal segment");
    assert_eq!(score, width as usize);
    assert_eq!(start, [0.0, 1.0]);
    assert_eq!(end, [(width - 1) as f32, 1.0]);

    let placed = place("Wide", &canvas, &[1], &TestFont::new()).expect("wide raster placement");
    assert_eq!(placed.glyphs.len(), 4);

    let mut small = Canvas::new(20, 10);
    small.rect(1, 0, 0, 19, 9);
    let small_map = small.map();
    let clipped = best_run(
        &small_map,
        &area,
        &bridged,
        [5.0, -2.0e7],
        [5.0, 2.0e7],
        None,
    )
    .expect("finite steep candidate clips to the raster");
    assert_eq!(clipped.0, 10);
    assert_eq!(clipped.1, [5.0, 0.0]);
    assert_eq!(clipped.2, [5.0, 9.0]);

    assert!(
        best_run(
            &small_map,
            &area,
            &bridged,
            [-1.0e8, 5.0],
            [1.0e8, 5.0],
            None,
        )
        .is_none()
    );
    assert!(
        best_run(
            &small_map,
            &area,
            &bridged,
            [f32::MAX, 0.0],
            [0.0, 0.0],
            None,
        )
        .is_none()
    );

    let mut tiny = Canvas::new(2, 2);
    tiny.rect(1, 0, 0, 1, 1);
    let tiny_map = tiny.map();
    let extreme = (i64::MIN as f32, (1i64 << 62) as f32);
    let diag = best_run(
        &tiny_map,
        &area,
        &bridged,
        [extreme.0, extreme.0],
        [extreme.1, extreme.1],
        None,
    )
    .expect("extreme i64 diagonal clips without overflowing");
    assert_eq!(diag.0, 2);
    assert_eq!(diag.1, [0.0, 0.0]);
    assert_eq!(diag.2, [1.0, 1.0]);
}

#[test]
fn authored_color_runs_measure_only_drawable_glyphs() {
    let mut canvas = Canvas::new(200, 100);
    canvas.rect(1, 20, 30, 179, 69);
    let lower_green = [0.2, 0.8, 0.3, 1.0];
    let upper_green = [0.0, 1.0, 0.0, 1.0];
    let font = TestFont::with_kerning('A', 'B', -2.0)
        .with_color('g', lower_green)
        .with_color('G', upper_green);
    let coloured = place("a\u{11}gb\u{11}!c", &canvas, &[1], &font).expect("coloured");
    let drawn: String = coloured.glyphs.iter().map(|g| g.ch).collect();
    assert_eq!(drawn, "ABC");
    assert_eq!(
        coloured.glyphs.iter().map(|g| g.color).collect::<Vec<_>>(),
        vec![
            None,
            Some(viewer_native_gui::native_text::packed(lower_green)),
            None,
        ]
    );
    let mut square = Canvas::new(40, 40);
    square.rect(1, 0, 0, 39, 39);
    let plain = place("ABAB", &square, &[1], &font).expect("plain");
    let coloured_pair = place("A\u{11}GB\u{11}!AB", &square, &[1], &font).expect("coloured pair");
    let pair_drawn: String = coloured_pair.glyphs.iter().map(|g| g.ch).collect();
    assert_eq!(pair_drawn, "ABAB");
    assert!(close(coloured_pair.scale, plain.scale * (20.0 / 22.0), 1e-4));
    assert_eq!(
        coloured_pair.glyphs.iter().map(|g| g.color).collect::<Vec<_>>(),
        vec![
            None,
            Some(viewer_native_gui::native_text::packed(upper_green)),
            None,
            None,
        ]
    );
}
