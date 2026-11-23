use super::*;
use crate::test_support::province_geometry::*;

#[test]
fn nearest_owned_pixel_uses_world_row_then_column_tie_order() {
    let map = fixture_map(3, 3, &[0, 1, 0, 1, 0, 1, 0, 1, 0]);
    let geometry = Centers::build(&map, &centroids(&[(1, [1, 1])]));
    assert_eq!(geometry.centers[&1], [1, 0]);
    let map = fixture_map(3, 1, &[1, 0, 1]);
    let geometry = Centers::build(&map, &centroids(&[(1, [1, 0])]));
    assert_eq!(geometry.centers[&1], [0, 0]);
}

#[test]
fn province_geometry_keeps_integer_centroids_and_directed_borders() {
    let map = MapData {
        width: 4,
        height: 2,
        province_ids: vec![1, 1, 2, 2, 1, 1, 2, 2],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    let geometry = ProvinceGeometry::build(&map);
    assert_eq!(geometry.centroid(1), Some([0.0, 0.0]));
    assert_eq!(geometry.centroid(2), Some([2.0, 0.0]));
    assert_eq!(geometry.provinces(), 2);
    let towards_two = &geometry.borders[&(1, 2)];
    assert!(towards_two.iter().all(|p| p[0] == 1 || p[0] == 0));
    assert!(
        geometry.borders[&(2, 1)]
            .iter()
            .all(|p| p[0] == 2 || p[0] == 3)
    );
}
