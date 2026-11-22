use super::*;

#[test]
fn heads_a_quarter_of_an_hour_apart_place_a_mover_four_different_places_at_sixty_million() {
    const HOUR: i32 = 60_800_000;
    let track = Track::new(&[Place { hour: HOUR, at: [0.0, 0.0, 0.0] }, Place { hour: HOUR + 48, at: [48.0, 0.0, 0.0] }], 0.0);
    let places: Vec<f32> = [0.0, 0.25, 0.5, 0.75]
        .into_iter()
        .map(|fraction| track.at(Head { hour: HOUR, fraction })[0])
        .collect();
    assert_eq!(places, vec![0.0, 0.25, 0.5, 0.75]);
    assert_eq!(track.at(Head { hour: HOUR + 1, fraction: 0.0 })[0], 1.0);
}

#[test]
fn a_mover_crossing_the_map_s_seam_travels_across_it_and_not_through_the_world() {
    const WIDTH: f32 = 1000.0;
    let track = Track::new(&[Place { hour: 100, at: [WIDTH - 1.0, 0.0, 5.0] }, Place { hour: 102, at: [1.0, 0.0, 5.0] }], WIDTH);
    let halfway = track.at(Head { hour: 101, fraction: 0.0 });
    assert_eq!(halfway, [0.0, 0.0, 5.0], "the seam itself, not {}", WIDTH / 2.0);
    assert_eq!(track.at(Head { hour: 100, fraction: 0.5 })[0], WIDTH - 0.5);
    assert_eq!(track.at(Head { hour: 101, fraction: 0.5 })[0], 0.5);
    let places: Vec<Place> = [0.0, 200.0, 400.0, 600.0].iter().zip(100..).map(|(&x, hour)| Place { hour, at: [x, 0.0, 0.0] }).collect();
    let track = Track::new(&places, WIDTH);
    assert_eq!(track.at(Head { hour: 100, fraction: 0.5 })[0], 100.0);
}

#[test]
fn a_head_outside_the_two_hours_holds_the_mover_at_an_end() {
    let track = Track::new(&[Place { hour: 100, at: [10.0, 0.0, 0.0] }, Place { hour: 148, at: [58.0, 0.0, 0.0] }], 0.0);
    assert_eq!(track.at(Head { hour: 90, fraction: 0.0 }), [10.0, 0.0, 0.0]);
    assert_eq!(track.at(Head { hour: 200, fraction: 0.0 }), [58.0, 0.0, 0.0]);
    assert_eq!(
        [
            track.outside(Head { hour: 90, fraction: 0.0 }),
            track.outside(Head { hour: 124, fraction: 0.0 }),
            track.outside(Head { hour: 200, fraction: 0.0 }),
        ],
        [-1, 0, 1]
    );
}

#[test]
fn a_head_behind_the_newest_places_reads_the_pair_that_brackets_it() {
    let places: Vec<Place> = (0..4)
        .map(|at| Place { hour: 60_800_000 + at * 48, at: [at as f32 * 48.0, 0.0, 0.0] })
        .collect();
    let track = Track::new(&places, 0.0);
    let at = |hour, fraction| track.at(Head { hour, fraction })[0];
    assert_eq!(at(60_800_000, 0.0), 0.0);
    assert_eq!(at(60_800_024, 0.0), 24.0);
    assert_eq!(at(60_800_048, 0.0), 48.0);
    assert_eq!(at(60_800_120, 0.0), 120.0, "between the third and the fourth");
    assert_eq!(track.outside(Head { hour: 60_800_060, fraction: 0.0 }), 0);
    assert_eq!(track.outside(Head { hour: 60_800_200, fraction: 0.0 }), 1);
}
