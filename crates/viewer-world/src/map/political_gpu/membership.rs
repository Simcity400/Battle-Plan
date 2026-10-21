//! Exact region areas retained by a layer whose province membership is unchanged.
use super::Province;
use std::collections::HashMap;

/// Nonzero keys include members with no pixels. Zero area is not absent
/// membership. Retained slots are used only after an exact source-key match.
pub(super) fn areas(
    region_of: &[u32],
    provinces: &[Province],
    retained: Option<(&HashMap<u32, usize>, &[u64])>,
) -> HashMap<u32, u64> {
    if let Some((keys, areas)) = retained {
        return keys
            .iter()
            .filter_map(|(&key, &slot)| (key != 0).then_some((key, areas[slot])))
            .collect();
    }
    let mut areas = HashMap::new();
    for (province, &key) in region_of.iter().enumerate() {
        if key != 0 {
            *areas.entry(key).or_default() += provinces[province].pixels;
        }
    }
    areas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn province(pixels: u64) -> Province {
        Province {
            pixels,
            bounds: [0; 4],
        }
    }

    #[test]
    fn retained_areas_keep_zero_pixel_keys_and_exact_integer_totals() {
        let provinces = [4, 30, 0, 50, 20, 99].map(province);
        let membership = [0, 17, 91, 29, 17, 0];
        let keys = [(0, 0), (17, 1), (91, 2), (29, 3)].into();
        let kept = [0, 50, 0, 50];
        let expected = [(17, 50), (29, 50), (91, 0)].into();
        assert_eq!(areas(&membership, &provinces, None), expected);
        assert_eq!(
            areas(&membership, &provinces, Some((&keys, &kept))),
            expected
        );
    }

    #[test]
    fn retained_areas_scale_to_5000_sparse_live_keys_without_slot_order_assumptions() {
        let provinces = (0..=5000).map(|id| province(id % 7)).collect::<Vec<_>>();
        let membership = std::iter::once(0)
            .chain((1..=5000).map(|id| u32::MAX - id * 3))
            .collect::<Vec<_>>();
        let mut keys = HashMap::from([(0, 0)]);
        let mut kept = vec![0; 5001];
        let mut expected = HashMap::new();
        for (id, &key) in membership.iter().enumerate().skip(1) {
            let slot = 5001 - id;
            keys.insert(key, slot);
            kept[slot] = id as u64 % 7;
            expected.insert(key, id as u64 % 7);
        }
        assert_eq!(expected.len(), 5000);
        assert_eq!(expected[&(u32::MAX - 21)], 0);
        assert_eq!(areas(&membership, &provinces, None), expected);
        assert_eq!(
            areas(&membership, &provinces, Some((&keys, &kept))),
            expected
        );
    }
}
