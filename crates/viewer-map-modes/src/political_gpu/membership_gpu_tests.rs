use super::*;

type MembershipStep = ([u32; 6], &'static [(u32, u64)]);

pub(super) fn retained_membership_keeps_exact_bytes_areas_and_refusals_across_hours(
    fallback: &Fallback,
) {
    let (width, height) = (64, 32);
    let harness = fallback.harness(width, height);
    let ids = (0..width * height)
        .map(|at| {
            if at % width < 24 {
                1
            } else if at / width < 16 {
                3
            } else {
                4
            }
        })
        .collect::<Vec<_>>();
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let field = FieldDefines::TEST;
    let blank = |_| [0; 3];
    let initial = Layer::new(LayerKind::Country, &[0; 6], &blank, &field);
    let mut gpu = PoliticalGpu::new(&harness.device, map, &initial).unwrap();
    let steps: [MembershipStep; 11] = [
        ([0; 6], &[]),
        (
            [0, 11, 91, 22, 33, 777],
            &[(11, 768), (22, 640), (33, 640), (91, 0)],
        ),
        (
            [0, 11, 91, 22, 33, 888],
            &[(11, 768), (22, 640), (33, 640), (91, 0)],
        ),
        ([0, 44, 91, 44, 33, 888], &[(33, 640), (44, 1408), (91, 0)]),
        ([0, 44, 91, 44, 33, 888], &[(33, 640), (44, 1408), (91, 0)]),
        ([0, 44, 0, 44, 33, 888], &[(33, 640), (44, 1408)]),
        ([0, 55, 91, 22, 55, 888], &[(22, 640), (55, 1408), (91, 0)]),
        ([0, 55, 91, 22, 55, 888], &[(22, 640), (55, 1408), (91, 0)]),
        ([0, 55, 91, 22, 55, 888], &[(22, 640), (55, 1408), (91, 0)]),
        ([0; 6], &[]),
        ([0; 6], &[]),
    ];
    for (hour, (keys, expected)) in steps.iter().enumerate() {
        let epoch = if hour == 8 { 7 } else { hour } as u32;
        let color = |key: u32| [key as u8, (epoch * 23) as u8, 173];
        let attributes = |key| map_display_data::landscape::Attributes {
            control: (key + epoch) as u8,
            contested: (epoch * 19) as u8,
            gradient: (!epoch.is_multiple_of(2)).then_some(83),
        };
        let style = |key: u32| gradient::RegionStyle {
            alpha: (255 - epoch * 17) as u8,
            pulse: !key.is_multiple_of(2),
            fill: if epoch.is_multiple_of(2) { 1.0 } else { 0.5 },
            thickness: (epoch % 3 == 2).then_some(7.0),
        };
        let mut layer = Layer::new(LayerKind::Country, keys, &color, &field);
        layer.region_attributes = Some(&attributes);
        layer.region_style = Some(&style);
        if hour == 7 || hour == 10 {
            gpu.refill();
        }
        gpu.update(&harness.device, &harness.queue, &layer).unwrap();
        assert_eq!(gpu.source_keys, keys[..5], "hour {hour}");
        let areas = gpu
            .keys
            .iter()
            .filter_map(|(&key, &slot)| (key != 0).then_some((key, gpu.areas[slot])))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            areas,
            expected.iter().copied().collect::<BTreeMap<_, _>>(),
            "hour {hour}"
        );
        for (province, &key) in keys[..5].iter().enumerate() {
            assert_eq!(
                gpu.mapping[province] as usize, gpu.keys[&key],
                "hour {hour}"
            );
        }
        for &(key, area) in *expected {
            let slot = gpu.keys[&key];
            assert_eq!(gpu.regions[slot].color, rgb(color(key)));
            assert_eq!(gpu.regions[slot].attributes, attributes(key).packed());
            assert_eq!(
                (gpu.regions[slot].fx, gpu.regions[slot].fill),
                style_words(style(key))
            );
            assert_eq!(
                gpu.thickness[slot],
                (
                    layer.thickness(key, area).to_bits(),
                    layer.center(key).to_bits()
                )
            );
        }
        let source = gpu.source_keys.clone();
        let mapping = gpu.mapping.clone();
        let areas = gpu.areas.clone();
        let regions = bytemuck::cast_slice::<Region, u8>(&gpu.regions).to_vec();
        let alpha = gpu.alpha.clone();
        let dirty = gpu.dirty.clone();
        let copies = gpu.copies.clone();
        let pending = gpu.pending;
        let invalid = Layer::new(LayerKind::Country, &keys[..4], &color, &field);
        let error = gpu
            .update(&harness.device, &harness.queue, &invalid)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "UnsupportedPoliticalCompute: region table no longer covers the map"
        );
        assert_eq!(gpu.source_keys, source);
        assert_eq!(gpu.mapping, mapping);
        assert_eq!(gpu.areas, areas);
        assert_eq!(bytemuck::cast_slice::<Region, u8>(&gpu.regions), regions);
        assert_eq!(gpu.alpha, alpha);
        assert_eq!(gpu.dirty, dirty);
        assert_eq!(gpu.copies, copies);
        assert_eq!(gpu.pending, pending);
        if hour == 3 {
            assert!(gpu.pending);
            continue;
        }
        let expected_pixels = gradient::build_layer(&map, &layer).unwrap();
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        gpu.encode(
            &mut encoder,
            [&harness.textures[0], &harness.textures[1]],
            0,
        );
        let pixels = harness.read(encoder, [&harness.textures[0], &harness.textures[1]], 0);
        gpu.recorded(true);
        compare(
            &expected_pixels,
            &pixels,
            &format!("fixed membership hour {hour}"),
        );
        if hour == 8 {
            assert_eq!(gpu.dispatched_cells, 0);
        }
    }
}
