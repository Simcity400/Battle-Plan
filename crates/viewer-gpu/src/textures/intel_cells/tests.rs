use super::*;
use content_reader_map::map::{CountryColors, ProvinceDefinition};

fn map(width: u32, height: u32, province: impl Fn(u32, u32) -> u32) -> MapData {
    let mut province_ids = Vec::new();
    for row in 0..height {
        for x in 0..width {
            province_ids.push(province(x, height - 1 - row));
        }
    }
    let definitions = [
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Sea),
        (4, ProvinceKind::Lake),
        (5, ProvinceKind::Land),
    ]
    .into_iter()
    .map(|(id, kind)| {
        (
            id,
            ProvinceDefinition {
                id,
                kind,
                rgb: [0; 3],
                continent: 0,
            },
        )
    })
    .collect();
    MapData {
        width,
        height,
        province_ids,
        definitions,
        country_colors: CountryColors::default(),
    }
}

fn scalar(map: &MapData, visibility: Option<&BTreeMap<u32, u8>>) -> (Vec<u8>, bool) {
    let mut texels = Vec::new();
    for z in 0..map.height / FOW_BLOCK {
        for x in 0..map.width / FOW_BLOCK {
            let mut sum = 0;
            for dz in 0..FOW_BLOCK {
                for dx in 0..FOW_BLOCK {
                    let row = map.height - 1 - (z * FOW_BLOCK + dz);
                    let at = (row * map.width + x * FOW_BLOCK + dx) as usize;
                    let province = map.province_ids[at];
                    let byte = map
                        .definitions
                        .get(&province)
                        .filter(|definition| definition.kind == ProvinceKind::Land)
                        .and_then(|_| visibility.and_then(|bytes| bytes.get(&province)))
                        .copied()
                        .unwrap_or(255);
                    sum += u32::from(byte);
                }
            }
            texels.push(!((sum / (FOW_BLOCK * FOW_BLOCK)) as u8));
        }
    }
    (texels, visibility.is_some())
}

fn assert_same(map: &MapData, cells: &IntelCells, visibility: Option<&BTreeMap<u32, u8>>) {
    let built = cells.intel_map(visibility);
    let (expected, fog) = scalar(map, visibility);
    assert_eq!(built.texels, expected);
    assert_eq!(built.fog, fog);
    let fogged = expected.iter().filter(|&&byte| byte > 0).count();
    assert_eq!(built.fogged_texels(), fogged);
    if visibility.is_none() {
        let clear = super::super::clear_intel_map(map).unwrap();
        assert_eq!(clear.texels, expected);
        assert!(!clear.fog);
        assert_eq!(clear.fogged_texels(), 0);
    }
}

#[test]
fn intel_cells_preserve_fixed_mixed_coast_bytes_and_floor_edges() {
    let map = map(13, 13, |x, z| {
        if x == 12 || z == 12 {
            return 1;
        }
        match (x / 6, z / 6) {
            (0, 0) => match z {
                0..=2 => 1,
                3 => 2,
                4 => 3,
                _ => {
                    if x < 3 {
                        2
                    } else {
                        u32::MAX
                    }
                }
            },
            (1, 0) => 5,
            (0, 1) => match z {
                6..=7 => 1,
                8..=9 => 2,
                _ => u32::MAX,
            },
            _ => {
                if z < 9 {
                    3
                } else {
                    4
                }
            }
        }
    });
    let cells = IntelCells::new(&map).unwrap();
    let visibility = BTreeMap::from([(1, 0), (2, 127), (3, 0), (4, 0), (u32::MAX, 0)]);
    assert_same(&map, &cells, Some(&visibility));
    let built = cells.intel_map(Some(&visibility));
    assert_eq!(built.texels, [160, 0, 128, 0]);
    assert_eq!(built.fogged_texels(), 2);
    assert_eq!(cells.offsets.len(), 5);
    assert_eq!(cells.entries.len(), 5);
}

#[test]
fn intel_cells_match_scalar_over_hours_fog_states_and_edge_sizes() {
    for [width, height] in [[1, 1], [5, 17], [6, 6], [7, 13], [13, 17], [25, 19]] {
        let map = map(width, height, |x, z| match (x * 7 + z * 11) % 9 {
            0 | 1 => 1,
            2 | 3 => 2,
            4 => 3,
            5 => 4,
            6 => 5,
            7 => 0,
            _ => u32::MAX,
        });
        let cells = IntelCells::new(&map).unwrap();
        for hour in 0..384u32 {
            let visibility = match hour % 13 {
                0 => None,
                1 => Some(BTreeMap::new()),
                _ => Some(BTreeMap::from([
                    (1, hour as u8),
                    (2, hour.wrapping_mul(73) as u8),
                    (3, 0),
                    (4, 0),
                    (u32::MAX, 0),
                ])),
            };
            assert_same(&map, &cells, visibility.as_ref());
        }
    }
}

#[test]
fn intel_cells_preserve_every_byte_and_none_versus_empty() {
    let map = map(6, 6, |_, _| 1);
    let cells = IntelCells::new(&map).unwrap();
    for byte in 0..=255u8 {
        let visibility = BTreeMap::from([(1, byte)]);
        let built = cells.intel_map(Some(&visibility));
        assert_eq!(built.texels, [!byte]);
        assert_eq!(built.fogged_texels(), usize::from(byte < 255));
        assert_same(&map, &cells, Some(&visibility));
    }
    let clear = cells.intel_map(None);
    let empty = cells.intel_map(Some(&BTreeMap::new()));
    assert_eq!(clear.texels, [0]);
    assert_eq!(empty.texels, clear.texels);
    assert!(!clear.fog);
    assert!(empty.fog);
    assert_eq!(clear.fogged_texels(), 0);
    assert_eq!(empty.fogged_texels(), 0);
}

#[test]
fn intel_cells_validate_before_clear_and_keep_original_diagnostics() {
    for (width, height, ids) in [(0, 6, 0), (6, 0, 0), (6, 6, 35), (6, 6, 37)] {
        let mut map = map(width, height, |_, _| 1);
        map.province_ids.resize(ids, 1);
        let cells_error = IntelCells::new(&map).err().unwrap();
        assert_eq!(cells_error.kind(), io::ErrorKind::InvalidData);
        let expected = format!("the province image is {width}x{height} but carries {ids} ids");
        assert_eq!(cells_error.to_string(), expected);
        let clear_error = super::super::clear_intel_map(&map).err().unwrap();
        assert_eq!(clear_error.kind(), cells_error.kind());
        assert_eq!(clear_error.to_string(), expected);
    }
}

#[test]
fn intel_cells_belong_to_each_content_and_size_from_actual_map() {
    let original = map(12, 6, |x, _| if x < 6 { 1 } else { 2 });
    let first = IntelCells::new(&original).unwrap();
    let mut changed = original.clone();
    changed.definitions.get_mut(&1).unwrap().kind = ProvinceKind::Sea;
    let second = IntelCells::new(&changed).unwrap();
    let visibility = BTreeMap::from([(1, 0), (2, 128)]);
    assert_same(&original, &first, Some(&visibility));
    assert_same(&changed, &second, Some(&visibility));
    assert_eq!(first.intel_map(Some(&visibility)).texels, [255, 127]);
    assert_eq!(second.intel_map(Some(&visibility)).texels, [0, 127]);
    assert_eq!(first.offsets.len(), 3);
    assert_eq!(first.entries.len(), 2);
    assert_eq!(second.entries.len(), 1);
    assert_eq!(first.land_provinces.len(), 3);
    assert_eq!(second.land_provinces.len(), 2);
    assert_eq!(size_of::<CellProvince>(), 8);
    let bytes = first.offsets.len() * size_of::<usize>()
        + first.entries.len() * size_of::<CellProvince>()
        + first.land_provinces.len() * size_of::<u32>();
    assert_eq!(
        bytes,
        3 * size_of::<usize>() + 2 * size_of::<CellProvince>() + 3 * 4
    );
}

fn assert_same_table(left: &IntelCells, right: &IntelCells) {
    assert_eq!(left.offsets, right.offsets);
    assert_eq!(left.land_provinces, right.land_provinces);
    let entries = |cells: &IntelCells| {
        cells
            .entries
            .iter()
            .map(|entry| (entry.land_index, entry.pixels))
            .collect::<Vec<_>>()
    };
    assert_eq!(entries(left), entries(right));
}

#[test]
fn intel_cells_prewarm_and_unstarted_fallback_keep_exact_tables_and_errors() {
    let map = Arc::new(map(13, 17, |x, z| (x + z * 3) % 7));
    let expected = IntelCells::new(&map).unwrap();
    let lazy = IntelCellsCache::new(Arc::clone(&map));
    assert!(lazy.cells.get().is_none());
    let warmed = IntelCellsCache::prewarm(Arc::clone(&map));
    for cells in [lazy.get_or_init().unwrap(), warmed.get_or_init().unwrap()] {
        assert_same_table(cells, &expected);
        for hour in 0..384u32 {
            let visibility = match hour % 13 {
                0 => None,
                1 => Some(BTreeMap::new()),
                _ => Some(BTreeMap::from([
                    (1, hour as u8),
                    (2, hour.wrapping_mul(73) as u8),
                ])),
            };
            assert_same(&map, cells, visibility.as_ref());
        }
    }
    let mut invalid = (*map).clone();
    invalid.province_ids.pop();
    let invalid = Arc::new(invalid);
    let lazy = IntelCellsCache::new(Arc::clone(&invalid));
    let warmed = IntelCellsCache::prewarm(invalid);
    for cache in [&lazy, warmed.as_ref()] {
        for _ in 0..2 {
            let error = cache.get_or_init().err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                "the province image is 13x17 but carries 220 ids"
            );
        }
    }
}

#[test]
fn intel_cells_pending_and_ready_prewarm_race_share_one_exact_table() {
    use std::sync::mpsc;
    let map = Arc::new(map(13, 17, |x, z| (x * 7 + z * 11) % 7));
    let cache = Arc::new(IntelCellsCache::new(Arc::clone(&map)));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let pending = Arc::clone(&cache);
    let prewarm = std::thread::spawn(move || {
        let _ = pending.get_or_init_with(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            IntelCells::new(&pending.map)
        });
        pending
    });
    entered_rx.recv().unwrap();
    assert!(cache.cells.get().is_none());
    let pending = Arc::clone(&cache);
    let (demand_tx, demand_rx) = mpsc::channel();
    let demand = std::thread::spawn(move || {
        demand_tx.send(()).unwrap();
        let cells = pending.get_or_init().unwrap();
        let visibility = BTreeMap::from([(1, 0), (2, 127)]);
        assert_same(&pending.map, cells, Some(&visibility));
        pending
    });
    demand_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    let warmed = prewarm.join().unwrap();
    let demanded = demand.join().unwrap();
    let ready = cache.get_or_init().unwrap();
    assert!(std::ptr::eq(ready, warmed.get_or_init().unwrap()));
    assert!(std::ptr::eq(ready, demanded.get_or_init().unwrap()));
    assert_same_table(ready, &IntelCells::new(&map).unwrap());
}

#[test]
fn intel_cells_allocation_failure_is_retryable_but_structural_refusal_is_retained() {
    let cache = IntelCellsCache::new(Arc::new(map(6, 6, |_, _| 1)));
    let error = cache
        .get_or_init_with(|| {
            let mut storage = Vec::<u8>::new();
            engine_native::allocation::reserve_exact(&mut storage, usize::MAX)?;
            unreachable!("an unaddressable reservation cannot succeed")
        })
        .err()
        .unwrap();
    assert!(engine_native::allocation::is_allocation_error(&error));
    assert!(cache.cells.get().is_none());
    let ready = cache.get_or_init().unwrap();
    assert_eq!(
        ready.intel_map(Some(&BTreeMap::from([(1, 0)]))).texels,
        [255]
    );
    assert!(std::ptr::eq(
        ready,
        cache
            .get_or_init_with(|| panic!("ready table rebuilt"))
            .unwrap()
    ));

    let invalid = IntelCellsCache::new(Arc::new(map(0, 6, |_, _| 1)));
    let expected = invalid.get_or_init().err().unwrap();
    let retained = invalid
        .get_or_init_with(|| panic!("structural refusal retried"))
        .err()
        .unwrap();
    assert_eq!(retained.kind(), expected.kind());
    assert_eq!(retained.to_string(), expected.to_string());
}

#[test]
fn intel_cell_patches_match_independent_scalar_across_pages_and_missing_keys() {
    let page = map_display_data::PAGE_ITEMS as u32;
    let map = map((2 * page + 7) * FOW_BLOCK, FOW_BLOCK, |x, _| {
        if x < page * FOW_BLOCK {
            1
        } else if x < 2 * page * FOW_BLOCK {
            2
        } else {
            3
        }
    });
    let cells = IntelCells::new(&map).unwrap();
    let mut visibility = BTreeMap::from([(1, 0), (2, 127)]);
    let mut previous = cells.intel_map(Some(&visibility));
    for step in 0..12 {
        let mut next = visibility.clone();
        match step {
            0 => {
                next.insert(2, 128);
            }
            1 => {
                next.insert(3, 0);
                next.insert(u32::MAX, 0);
            }
            2 => {
                next.remove(&2);
            }
            3 => {
                next.insert(2, 0);
            }
            4 => next.clear(),
            5 => {
                next.insert(1, 250);
            }
            _ => {
                next.insert(2, (step * 19) as u8);
            }
        }
        let actual = cells.patch(&previous, &visibility, &next);
        let (expected, fog) = scalar(&map, Some(&next));
        assert_eq!(actual.texels, expected, "step {step}");
        assert_eq!(actual.fog, fog);
        assert_eq!(
            actual.fogged_texels(),
            expected.iter().filter(|&&byte| byte != 0).count()
        );
        if step == 0 {
            assert_eq!(actual.texels.changed_pages(&previous.texels), [1]);
        }
        if step == 1 {
            assert!(actual.texels.ptr_eq(&previous.texels));
        }
        previous = actual;
        visibility = next;
    }
}
