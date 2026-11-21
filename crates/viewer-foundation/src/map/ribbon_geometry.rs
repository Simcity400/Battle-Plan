pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn length(v: [f32; 3]) -> f32 {
    (v[1] * v[1] + v[0] * v[0] + v[2] * v[2]).sqrt()
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l != 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}

fn side(v: [f32; 3]) -> [f32; 3] {
    [-v[2], v[1], v[0]]
}

pub fn smooth_sharp_turns(points: &mut Vec<[f32; 3]>, amount: f32) {
    let mut index = 2usize;
    while (index as i64) < points.len() as i64 - 1 {
        let (previous, current, next) = (points[index - 1], points[index], points[index + 1]);
        let incoming = sub(previous, current);
        let half_in = length(incoming) * 0.5;
        let a = normalize(incoming);
        let outgoing = sub(next, current);
        let half_out = length(outgoing) * 0.5;
        let b = normalize(outgoing);
        if b[0] * a[0] + b[1] * a[1] + b[2] * a[2] <= 0.5 {
            let count = points.len();
            let mut later = index + 1;
            while later < count - 1 {
                if points[later] == current {
                    let push = side(normalize(outgoing));
                    let last = later == count - 2;
                    let reach = if last { amount + amount } else { amount };
                    for (value, push) in points[index].iter_mut().zip(push) {
                        *value += push * reach;
                    }
                    if !last {
                        let away = side(normalize(sub(points[later + 1], current)));
                        for (value, away) in points[later].iter_mut().zip(away) {
                            *value += away * amount;
                        }
                    }
                    break;
                }
                later += 1;
            }
        } else {
            let turned = 0.0 < (b[0] * a[2] - b[2] * a[0]);
            let inset = |direction: [f32; 3], half: f32, towards: [f32; 3], out: bool| {
                let corner = [
                    direction[0] * half * 0.5 + current[0],
                    direction[1] * half * 0.5 + current[1],
                    direction[2] * half * 0.5 + current[2],
                ];
                let mut along = if out {
                    sub(towards, corner)
                } else {
                    sub(corner, towards)
                };
                if turned {
                    along = [-along[0], -along[1], -along[2]];
                }
                let push = side(normalize(along));
                [
                    push[0] * amount * 3.0 + corner[0],
                    push[1] * amount * 3.0 + corner[1],
                    push[2] * amount * 3.0 + corner[2],
                ]
            };
            let before = inset(a, half_in, previous, false);
            let after = inset(b, half_out, next, true);
            points.insert(index, before);
            index += 2;
            points.insert(index, after);
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_sharp_turn_gets_two_pushed_points_and_a_straight_run_none() {
        let straight = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
        ];
        let mut points = straight.clone();
        smooth_sharp_turns(&mut points, 1.0);
        assert_eq!(points, straight);
        let mut turn = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [10.0, 0.0, 1.0],
            [0.0, 0.0, 2.0],
        ];
        smooth_sharp_turns(&mut turn, 1.0);
        assert_eq!(turn.len(), 7);
        assert_eq!(turn[2], [17.5, 0.0, -3.0]);
        assert_eq!(turn[3], [20.0, 0.0, 0.0]);
    }

    #[test]
    fn a_revisited_point_is_pushed_apart_at_both_visits() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [20.0, 0.0, 10.0],
            [30.0, 0.0, 10.0],
            [30.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [20.0, 0.0, -10.0],
            [20.0, 0.0, -20.0],
        ];
        smooth_sharp_turns(&mut points, 1.0);
        assert_eq!(points.len(), 9);
        assert_eq!(points[2], [19.0, 0.0, 0.0]);
        assert_eq!(points[6], [21.0, 0.0, 0.0]);
    }
}
