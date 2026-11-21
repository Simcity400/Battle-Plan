use super::*;

fn visible([gx0, gz0, gx1, gz1]: [f32; 4], offset: f32, [x0, z0, x1, z1]: [f32; 4]) -> bool {
    gx0 + offset <= x1 && gx1 + offset >= x0 && gz0 <= z1 && gz1 >= z0
}

struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn group(numbers: &mut Numbers) -> [f32; 4] {
    let (x, z) = (numbers.next() * 5600., numbers.next() * 2000.);
    let (w, d) = (numbers.next() * 40. + 0.5, numbers.next() * 40. + 0.5);
    [x, z, x + w, z + d]
}

fn scanned(bounds: &[[f32; 4]], offsets: &[f32], rectangle: [f32; 4], visited: &[bool]) -> Vec<usize> {
    let groups = bounds.len();
    (0..groups)
        .filter(|&group| {
            offsets
                .iter()
                .enumerate()
                .any(|(view, &offset)| visible(bounds[group], offset, rectangle) != visited[view * groups + group])
        })
        .collect()
}

#[test]
fn the_groups_whose_view_flipped_are_found_among_the_candidates_alone() {
    let mut numbers = Numbers(7);
    let mut bounds: Vec<[f32; 4]> = (0..3000).map(|_| group(&mut numbers)).collect();
    let offsets = [-5632., 0., 5632.];
    let views = offsets.len();
    let mut grid = Grid::default();
    let mut centre = [180., 360.];
    let mut rectangle = [100., 300., 260., 420.];
    let mut visited: Vec<bool> = offsets
        .iter()
        .flat_map(|&offset| bounds.iter().map(move |&bounds| visible(bounds, offset, rectangle)))
        .collect();
    let mut tested = 0;
    for frame in 1..400u64 {
        centre = [centre[0] + numbers.next() * 60. - 20., (centre[1] + numbers.next() * 30. - 15.).clamp(100., 1900.)];
        if centre[0] > 5632. {
            centre[0] -= 5632.;
        }
        let (width, depth) = (120. + numbers.next() * 80., 80. + numbers.next() * 60.);
        rectangle = [centre[0] - width / 2., centre[1] - depth / 2., centre[0] + width / 2., centre[1] + depth / 2.];
        let mut moved: Vec<usize> = (0..5).map(|_| (numbers.next() * 3000.) as usize % 3000).collect();
        for &at in &moved {
            bounds[at] = group(&mut numbers);
        }
        if frame == 200 {
            bounds[moved[0]] = [6000., 2500., 6010., 2510.];
        }
        moved.sort_unstable();
        moved.dedup();
        let field = FieldFrame { frame: Some(frame), since: Some(frame - 1), whole: false, moved: &moved };
        grid.sync(bounds.len(), |at| bounds[at], field);
        let expected = scanned(&bounds, &offsets, rectangle, &visited);
        let mut found = Vec::new();
        assert!(grid.candidates(rectangle, &offsets, &mut found));
        let groups = bounds.len();
        found.extend(visited.iter().enumerate().filter(|(_, drawn)| **drawn).map(|(at, _)| at % groups));
        found.extend(&moved);
        found.sort_unstable();
        found.dedup();
        tested += found.len();
        found.retain(|&group| {
            offsets
                .iter()
                .enumerate()
                .any(|(view, &offset)| visible(bounds[group], offset, rectangle) != visited[view * groups + group])
        });
        assert_eq!(found, expected, "frame {frame}");
        for &group in &found {
            for (view, &offset) in offsets.iter().enumerate() {
                visited[view * groups + group] = visible(bounds[group], offset, rectangle);
            }
        }
        assert_eq!(visited.len(), views * groups);
    }
    assert!(tested < 399 * 3000 / 4, "a low eye tests a small share of the groups: {tested}");
}

#[test]
fn a_grid_that_cannot_follow_places_every_group_again_or_refuses() {
    let bounds = vec![[0., 0., 1., 1.], [10., 10., 11., 11.]];
    let mut grid = Grid::default();
    grid.sync(2, |at| bounds[at], FieldFrame { frame: Some(5), since: Some(4), whole: false, moved: &[] });
    let mut out = Vec::new();
    assert!(grid.candidates([9., 9., 12., 12.], &[0.], &mut out));
    assert!(out.contains(&1));
    let moved = vec![[0., 0., 1., 1.], [0.5, 0.5, 0.75, 0.75]];
    grid.sync(2, |at| moved[at], FieldFrame { frame: Some(7), since: Some(6), whole: false, moved: &[] });
    out.clear();
    assert!(grid.candidates([0.6, 0.6, 0.7, 0.7], &[0.], &mut out));
    assert!(out.contains(&1));
    assert!(!grid.candidates([f32::NAN, 0., 1., 1.], &[0.], &mut out));
    let broken = vec![[0., 0., f32::INFINITY, 1.]];
    grid.sync(1, |at| broken[at], FieldFrame { frame: Some(8), since: Some(7), whole: true, moved: &[] });
    assert!(!grid.candidates([0., 0., 1., 1.], &[0.], &mut out));
}
