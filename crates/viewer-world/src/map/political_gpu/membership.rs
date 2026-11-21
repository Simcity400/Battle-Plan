use super::Province;
use std::collections::HashMap;

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
}
