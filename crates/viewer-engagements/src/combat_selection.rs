use viewer_runtime::geometry::contains;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Land(u64),
    Naval(u64),
    NavalResult(u64),
    AirRegion(u32),
    State(u32),
    NavalRegion(u32),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub selection: Selection,
    pub rect: [f32; 4],
    pub layer: u64,
    pub order: usize,
    pub supported: bool,
}
impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }
}

#[cfg(test)]
mod scale_tests {
    use viewer_gpu::ui_scale::device_corners;
    use viewer_gpu::ui_scale::to_interface;
    #[test]
    fn a_click_hits_exactly_what_the_scaled_quad_covers() {
        let hit = super::Hit {
            selection: super::Selection::Land(7),
            rect: [100.0, 100.0, 40.0, 20.0],
            layer: 0,
            order: 0,
            supported: true,
        };
        let [x0, y0, x1, y1] = device_corners(hit.rect, 1.25);
        assert_eq!([x0, y0, x1, y1], [125.0, 125.0, 175.0, 150.0]);
        let click = |device: [f32; 2]| hit.contains(to_interface(device, 1.25));
        assert!(click([x0, y0]));
        assert!(click([x1 - 0.5, y1 - 0.5]));
        assert!(!click([x1, y0]));
        assert!(!click([x0, y1]));
        assert!(!click([x0 - 0.5, y0]));
        assert!(!hit.contains([170.0, 140.0]));
        assert!(click([170.0, 140.0]));
    }
}
