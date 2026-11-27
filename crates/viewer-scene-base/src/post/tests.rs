use super::*;
use content_reader_map::map::ProvinceKind;
use viewer_gpu::camera::MapDefines;

const BERLIN_DAY: i32 = 60832933;

fn camera(center: [f32; 2], height: f32) -> Camera {
    Camera {
        center,
        height,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    }
}

fn named(lut: usize, middle_grey: f32) -> PostValues {
    PostValues {
        lut,
        middle_grey,
        hdr_min: 0.5,
        hdr_max: 1.0,
        bloom: [1.5, 0.9, 0.4],
    }
}

fn resolved(lut: usize, middle_grey: f32) -> PostValuesResolved {
    PostValuesResolved {
        lut: LutSource::Asset(lut),
        middle_grey,
        hdr_min: 0.5,
        hdr_max: 1.0,
        bloom: [1.5, 0.9, 0.4],
    }
}

fn height(name: &str, height: f32, fade_distance: f32) -> HeightVolume {
    HeightVolume {
        day: name.into(),
        night: name.into(),
        height,
        fade_distance,
    }
}

fn volume() -> BoxVolume {
    BoxVolume {
        day: "green".into(),
        night: "green_night".into(),
        day_winter: Some("winter_day".into()),
        night_winter: Some("winter_night".into()),
        position: [3300.0, 275.0, 1700.0],
        size: [1300.0, 800.0, 700.0],
        fade_distance: 125.0,
        records: Vec::new(),
    }
}

fn map(width: u32, top_down: &[u32], kinds: &[(u32, ProvinceKind)]) -> MapData {
    MapData {
        width,
        height: top_down.len() as u32 / width,
        province_ids: top_down.to_vec(),
        definitions: kinds
            .iter()
            .map(|&(id, kind)| {
                let rgb = [id as u8, 0, 0];
                (
                    id,
                    content_reader_map::map::ProvinceDefinition {
                        id,
                        rgb,
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: Default::default(),
    }
}

fn extent(min: [i32; 2], size: [i32; 2], area: i32, land: bool) -> Option<ProvinceExtent> {
    Some(ProvinceExtent {
        min,
        size,
        area,
        land,
    })
}

#[test]
fn a_kept_block_stands_at_the_devices_offset_alignment_and_zeroes_what_follows_its_bytes() {
    for alignment in [1_u32, 16, 64, 256, 512] {
        let mut blocks = Blocks::new(alignment);
        let first = [1_u8, 2, 3];
        let second = [4_u8; 80];
        let at_first = blocks.push(&first);
        let at_second = blocks.push(&second);
        let (at_first, at_second) = (at_first as usize, at_second as usize);
        assert_eq!(at_first, 0);
        assert_eq!(at_second % alignment as usize, 0);
        assert!(at_second >= BLOCK, "{at_second} leaves a register short");
        assert_eq!(blocks.bytes[..first.len()], first);
        assert!(blocks.bytes[first.len()..at_second].iter().all(|byte| *byte == 0));
        assert_eq!(blocks.bytes[at_second..at_second + second.len()], second);
        assert!(
            blocks.bytes[at_second + second.len()..]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(blocks.bytes.len() - at_second >= BLOCK);
    }
}

#[test]
fn luminance_reduces_in_fixed_targets_and_gathers_the_last_rectangle() {
    let schedule = ReductionSchedule::new([2048, 1152]);
    assert_eq!(schedule.seed, [1024, 576]);
    assert_eq!(schedule.bloom, [512, 288]);
    assert_eq!(schedule.backing, [512, 288]);
    let viewports: Vec<_> = schedule.steps.iter().map(|step| step.viewport).collect();
    assert_eq!(
        viewports,
        [
            [512, 288],
            [256, 144],
            [128, 72],
            [64, 36],
            [32, 18],
            [16, 9],
            [8, 5],
            [4, 3]
        ]
    );
    assert_eq!(schedule.gather, [4, 3]);
    assert_eq!(schedule.steps[6].uv_scale[1], (10.0 / 9.0) * (9.0 / 288.0));

    let odd = ReductionSchedule::new([1023, 575]);
    assert_eq!((odd.seed, odd.backing), ([511, 287], [256, 144]));
    assert_eq!(odd.bloom, [256, 144]);
    assert_eq!(ReductionSchedule::new([1920, 1080]).bloom, [480, 270]);
    assert!(odd.steps[0].uv_scale[0] > 1.0);
    let half = ReductionSchedule::new([1025, 577]);
    assert_eq!((half.seed, half.backing), ([512, 288], [256, 144]));
    let tiny = ReductionSchedule::new([12, 10]);
    assert!(tiny.steps.is_empty());
    assert_eq!(tiny.gather, [6, 5]);
}

#[test]
fn bloom_pairs_sum_to_the_scale_between_their_texels_and_a_zero_scale_has_no_blur() {
    let mut volumes = europe();
    let eye = camera([3010.0, 1568.0], 400.0);
    let [width, scale, _] = volumes.select(eye, DayNight::at_hour(BERLIN_DAY)).values.bloom;
    let (centre, weights, offsets) =
        blur_weights(width, scale).expect("a volume whose bloom scale is not zero draws its blurs");
    assert!((centre + 2.0 * weights.iter().sum::<f32>() - scale).abs() < 1e-6);
    for (pair, offset) in offsets.iter().enumerate() {
        assert!((2 * pair + 1) as f32 <= *offset && *offset <= (2 * pair + 2) as f32);
    }

    for entry in volumes.values.values_mut() {
        entry.bloom[1] = 0.0;
    }
    let [width, scale, _] = volumes.select(eye, DayNight::at_hour(BERLIN_DAY)).values.bloom;
    assert!(width > 0.0);
    assert_eq!(scale, 0.0);
    assert!(blur_weights(width, scale).is_none());
}

#[test]
fn the_last_grade_holds_only_for_sub_threshold_night_changes() {
    let previous = Selection {
        night: 0.5,
        ..Selection::default()
    };
    let with = |night, snow| Selection {
        night,
        snow,
        ..previous
    };
    assert!(with(0.5005, None).keeps(previous));
    assert!(!with(0.5015, None).keeps(previous));
    assert!(!with(0.5, Some(0.00001)).keeps(previous));
}

#[test]
fn the_identity_cube_is_red_across_blue_slices_with_nearest_bytes() {
    let lut = identity_lut();
    assert_eq!((lut.width, lut.height), (1024, 32));
    let pixel = |red: usize, green: usize, blue: usize| {
        let at = (green * 1024 + blue * 32 + red) * 4;
        &lut.pixels[at..at + 4]
    };
    assert_eq!(pixel(0, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(1, 2, 31), [8, 16, 255, 255]);
    assert_eq!(pixel(31, 31, 31), [255; 4]);
}

#[test]
fn the_night_factor_follows_the_eye_and_applies_strength_before_saturating() {
    let eye = [3010.0, 310.178, 1538.0];
    let defines = PostDefines::default();
    assert_eq!(
        night_factor(eye, MapSize::TEST, DayNight::at_hour(BERLIN_DAY), defines),
        0.0
    );
    assert_eq!(
        night_factor(
            eye,
            MapSize::TEST,
            DayNight::at_hour(BERLIN_DAY + 12),
            defines
        ),
        1.0
    );
    let equator = [
        defines.gmt_offset,
        300.0,
        2048.0 * (0.5 - defines.poles[0]) / (defines.poles[1] - defines.poles[0]),
    ];
    let half = DayNight {
        hour_fraction: 0.0,
        sun_direction: [0.0, 0.0, 1.0],
        strength: 0.5,
    };
    assert_eq!(night_factor(equator, MapSize::TEST, half, defines), 1.0);
    assert_eq!(
        night_factor(
            eye,
            MapSize::TEST,
            DayNight::at_hour(BERLIN_DAY + 12).with_cycle(false),
            defines
        ),
        0.0
    );
}

#[test]
fn a_box_fades_in_from_its_faces() {
    assert_eq!(box_fade([3010.0, 310.178, 1538.0], &volume()), 1.0);
    assert_eq!(box_fade([4000.0, 310.0, 1538.0], &volume()), 0.0);
    let edge = box_fade([3010.0, 310.0, 1400.0], &volume());
    assert!((edge - 0.4).abs() < 1e-5, "{edge}");
}

#[test]
fn province_extents_count_pixels_in_the_bottom_up_raster_with_even_edges() {
    use ProvinceKind::{Land, Sea};
    let map = map(
        3,
        &[1, 1, 2, 2, 1, 3],
        &[(1, Land), (2, Land), (3, Sea), (4, Land)],
    );
    let extents = province_extents(&map);
    assert_eq!(extents.len(), 5);
    assert_eq!(extents[0], None, "an undefined id is no province");
    assert_eq!(extents[1], extent([0, 0], [2, 2], 3, true));
    assert_eq!(extents[2], extent([0, 0], [4, 2], 2, true));
    assert_eq!(extents[3], extent([2, 0], [2, 2], 1, false));
    assert_eq!(
        extents[4],
        extent([0, 0], [0, 0], 0, true),
        "a province without pixels keeps the constructor's zeros"
    );
}

#[test]
fn a_region_is_represented_by_its_last_eligible_province_and_weighted_by_whole_area() {
    let mut extents = vec![None; 12];
    extents[5] = extent([12, 12], [2, 2], 4, true);
    extents[6] = extent([15, 15], [15, 15], 100, true);
    extents[7] = extent([12, 12], [2, 2], 50, false);
    extents[8] = extent([20, 12], [2, 2], 50, true);
    extents[9] = extent([40, 40], [2, 2], 50, true);
    extents[10] = extent([12, 12], [0, 0], 0, true);
    extents[11] = extent([19, 10], [1, 1], 26, true);
    let regions: [&[u32]; 4] = [&[5, 6, 7, 8, 5, 99], &[9], &[10], &[11]];
    let records = snow_records([10.0, 10.0, 20.0, 20.0], regions, &extents);
    assert_eq!(
        records,
        [
            SnowRecord {
                province: 6,
                weight: 0.8
            },
            SnowRecord {
                province: 11,
                weight: 0.2
            },
        ]
    );
}

#[test]
fn the_factor_bands_each_representative_then_the_weighted_sum() {
    let records = [
        SnowRecord {
            province: 1,
            weight: 0.8,
        },
        SnowRecord {
            province: 2,
            weight: 0.2,
        },
    ];
    let defines = PostDefines::default();
    assert_eq!(
        map_snow::winter::snow_factor(
            &records,
            &[],
            SnowDefines {
                per_province: defines.snow_per_province,
                total: defines.snow_total
            }
        ),
        None
    );
    let factor = map_snow::winter::snow_factor(
        &records,
        &[Some(0), Some(10_250), Some(-5)],
        SnowDefines {
            per_province: defines.snow_per_province,
            total: defines.snow_total,
        },
    );
    assert!((factor.unwrap() - 0.4).abs() < 1e-4, "{factor:?}");
    assert_eq!(
        map_snow::winter::snow_factor(
            &records,
            &[Some(0), Some(0), Some(20_000)],
            SnowDefines {
                per_province: defines.snow_per_province,
                total: defines.snow_total
            }
        ),
        Some(1.0)
    );
    assert!((band(0.1005, 0.1, 0.1) - 0.5).abs() < 1e-3, "width floor");
    assert_eq!(band(f32::NAN, 0.0, 0.05), 0.0);
}

#[test]
fn a_volume_records_regions_against_world_z_rows() {
    use ProvinceKind::Land;
    let map = map(3, &[1, 1, 2, 2, 1, 3], &[(1, Land), (2, Land), (3, Land)]);
    let mut volume = volume();
    volume.position = [2.5, 0.0, 1.5];
    volume.size = [0.6, 1.0, 0.6];
    let mut volumes = Volumes {
        boxes: vec![volume],
        ..Volumes::default()
    };
    let region = StrategicRegion {
        id: 1,
        provinces: vec![2, 3],
        ..StrategicRegion::default()
    };
    volumes.set_regions(&map, &BTreeMap::from([(1, region)]));
    assert_eq!(
        volumes.boxes[0].records,
        [SnowRecord {
            province: 3,
            weight: 1.0
        }]
    );
    volumes
        .set_snow(&[Some(0), Some(0), Some(0), Some(20_000)])
        .unwrap();
    assert_eq!(&*volumes.snow, &[Some(1.0)]);
}

fn europe() -> Volumes {
    Volumes {
        identity: 9,
        values: BTreeMap::from([
            ("base".into(), named(0, 0.2)),
            ("high".into(), named(4, 0.6)),
            ("green".into(), named(1, 0.4)),
            ("green_night".into(), named(1, 0.4)),
            ("winter_day".into(), named(2, 0.8)),
            ("winter_night".into(), named(2, 0.8)),
        ]),
        heights: vec![height("base", 0.0, 0.0), height("high", 300.0, 200.0)],
        boxes: vec![volume()],
        snow: vec![None].into(),
        ..Volumes::default()
    }
}

#[test]
fn height_volumes_fade_between_neighbours_and_none_disables_boxes() {
    let day = DayNight::at_hour(BERLIN_DAY);
    let mut volumes = europe();
    volumes.boxes.clear();
    let grading = volumes.select(camera([3010.0, 1568.0], 400.0), day);
    assert!((grading.values.middle_grey - 0.4).abs() < 1e-6);
    assert_eq!(grading.luts.operations[0].factors, [0.0, 0.5]);

    let bare = Volumes {
        heights: Vec::new(),
        ..europe()
    };
    let grading = bare.select(camera([3010.0, 1568.0], 310.0), day);
    assert_eq!(grading.values.lut, LutSource::Asset(9));
    assert_eq!(grading.selection.region, None);
}

#[test]
fn the_first_box_around_the_eye_wins_and_unknown_snow_keeps_the_height_grade() {
    let day = DayNight::at_hour(BERLIN_DAY);
    let mut volumes = europe();
    let mut second = volume();
    second.day_winter = None;
    second.night_winter = None;
    second.day = "high".into();
    second.night = "high".into();
    volumes.boxes.push(second);
    volumes.snow = vec![None, None].into();
    let reference = camera([3010.0, 1568.0], 290.0);
    let unknown = volumes.select(reference, day);
    assert_eq!(unknown.selection.region, Some(0));
    assert_eq!(unknown.values, resolved(0, 0.2));
    volumes.snow[0] = Some(1.0);
    assert_eq!(volumes.select(reference, day).values.middle_grey, 0.8);
    volumes.snow[0] = Some(0.0);
    assert_eq!(volumes.select(reference, day).values.middle_grey, 0.4);
    let south = volumes.select(camera([3010.0, 1370.0], 290.0), day);
    assert_eq!(south.selection.region, None);
}

#[test]
fn lut_blends_skip_endpoints_and_store_each_native_draw() {
    let mut program = LutProgram::default();
    let [a, b, c, d] = [0, 1, 2, 3].map(|lut| resolved(lut, 0.2 * (lut + 1) as f32));
    assert_eq!(program.blend(a, b, 1.0), b);
    assert_eq!(program.blend(a, b, 0.0), a);
    assert_eq!(program.len, 0);
    let winter = program.compose([a, b, c, d], 0.25, 0.5);
    assert_eq!(program.len, 1);
    assert_eq!(program.operations[0].factors, [0.25, 0.5]);
    assert!(program.operations[0].double);
    let result = program.blend(a, winter, 0.75);
    assert_eq!(program.operations[1].sources[2], LutSource::Intermediate(0));
    assert!(!program.operations[1].double);
    assert_eq!(result.lut, LutSource::Intermediate(1));
    assert!((result.middle_grey - 0.3875).abs() < 1e-6);
}

#[test]
fn inherit_folds_parents_and_refuses_unknown_or_cyclic_chains() {
    let raw = |inherit: Option<&str>, lut: Option<&str>, middle_grey| RawValues {
        inherit: inherit.map(Into::into),
        lut: lut.map(Into::into),
        middle_grey,
        hdr_min: None,
        hdr_max: None,
        bloom: [None; 3],
    };
    let mut catalogue = BTreeMap::from([
        ("standard".to_string(), raw(None, Some("a.tga"), Some(0.55))),
        (
            "green".into(),
            raw(Some("standard"), Some("b.tga"), Some(0.3)),
        ),
        ("plain".into(), raw(Some("standard"), None, None)),
        ("bare".into(), raw(None, None, None)),
    ]);
    let defines = PostDefines::default();
    let mut paths = vec![String::new()];
    let green = resolve(&catalogue, "green", &mut paths, defines).unwrap();
    assert_eq!((green.lut, green.middle_grey, green.hdr_min), (1, 0.3, 0.5));
    let plain = resolve(&catalogue, "plain", &mut paths, defines).unwrap();
    assert_eq!((plain.lut, plain.middle_grey), (2, 0.55));
    let bare = resolve(&catalogue, "bare", &mut paths, defines).unwrap();
    assert_eq!((bare.lut, bare.middle_grey), (0, defines.middle_grey));
    assert_eq!(paths, ["", "b.tga", "a.tga"]);

    catalogue.insert("orphan".into(), raw(Some("missing"), None, None));
    assert!(resolve(&catalogue, "orphan", &mut paths, defines).is_err());
    catalogue.insert("standard".into(), raw(Some("green"), None, None));
    assert!(resolve(&catalogue, "green", &mut paths, defines).is_err());
}

#[test]
fn prepared_snow_factors_preserve_record_order_and_unknown_representatives() {
    let mut volumes = europe();
    volumes.boxes[0].records = vec![
        SnowRecord {
            province: 3,
            weight: 0.17,
        },
        SnowRecord {
            province: 1,
            weight: 0.61,
        },
        SnowRecord {
            province: 2,
            weight: 0.22,
        },
    ];
    let mut second = volume();
    second.records = vec![SnowRecord {
        province: 4,
        weight: 1.0,
    }];
    volumes.boxes.push(second);
    volumes.boxes.push(volume());
    volumes.defines.snow_total = [0.0, 1.0];
    for (snow, expected_bits) in [
        (vec![], [None, None, Some(0)]),
        (
            vec![None, Some(10_250), Some(-5), Some(16_777), None],
            [Some(0x3e05_970a), None, Some(0)],
        ),
        (
            vec![
                Some(0),
                Some(10_999),
                Some(20_000),
                Some(10_123),
                Some(100_000),
            ],
            [Some(0x3e90_e94f), Some(0x3f80_0000), Some(0)],
        ),
        (
            vec![
                Some(0),
                Some(i64::MAX),
                Some(i64::MIN),
                Some(19_999),
                Some(0),
            ],
            [Some(0x3f47_acf8), Some(0), Some(0)],
        ),
    ] {
        let before = volumes.snow.clone();
        let prepared = volumes.snow_factors(&snow).unwrap();
        assert!(
            before.ptr_eq(&volumes.snow),
            "preparation cannot mutate the shown factors"
        );
        assert_eq!(
            prepared
                .iter()
                .map(|v| v.map(f32::to_bits))
                .collect::<Vec<_>>(),
            expected_bits
        );
        volumes.set_snow(&snow).unwrap();
        assert_eq!(&*prepared, &*volumes.snow);
    }
}
