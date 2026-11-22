use viewer_runtime::unit_handle::Unit;

#[derive(Clone)]
pub struct Route {
    pub unit: Unit,
    pub province: u32,
    pub style: &'static [u8],
    pub controls: Vec<[f32; 3]>,
    pub total_arc: f32,
    pub fill_target: f32,
    pub free_arc: f32,
}

pub fn catmull_rom(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let (t2, t3) = (t * t, t * t * t);
    let a = ((t2 + t2) - t3) - t;
    let b = (3.0 * t3 - 5.0 * t2) + 2.0;
    let c = (4.0 * t2 - 3.0 * t3) + t;
    let d = t3 - t2;
    let mut out = [0.0f32; 3];
    for axis in 0..3 {
        out[axis] = (p0[axis] * a + p1[axis] * b + c * p2[axis] + d * p3[axis]) * 0.5;
    }
    out
}
