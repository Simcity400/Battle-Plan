use crate::controls::{flight, orbit};
use std::io;
use viewer_gpu::camera::{Free, Lens, Placement, Pose};

pub(crate) const LEGS: [(&str, f32); 5] = [
    ("tilted", 4.0),
    ("pan", 6.0),
    ("turn", 6.0),
    ("climb", 8.0),
    ("high", 3.0),
];

pub(crate) const CLIMB_TO: f32 = 600.0;

const LOW: f32 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Flight {
    pub start: [f32; 2],
    pub height: f32,
}

impl Flight {
    pub(crate) fn parse(value: &str) -> io::Result<Self> {
        const USAGE: &str = "--flight needs x,z[,height]: the start on the map and the height under the units' cutoff";
        let values = value
            .split(',')
            .map(|value| value.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?;
        match values.as_slice() {
            [x, z] => Ok(Self { start: [*x, *z], height: LOW }),
            [x, z, height] if *height > 0.0 => Ok(Self { start: [*x, *z], height: *height }),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE)),
        }
        .and_then(|flight| {
            if flight.start.iter().chain([&flight.height]).all(|value| value.is_finite()) {
                Ok(flight)
            } else {
                Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE))
            }
        })
    }

    pub(crate) fn at(&self, seconds: f64, map_width: f32) -> (&'static str, Placement) {
        let total: f32 = LEGS.iter().map(|(_, seconds)| seconds).sum();
        let mut left = (seconds as f32).rem_euclid(total);
        let speed = flight::FASTEST * flight::BOOST;
        let east = std::f32::consts::FRAC_PI_2;
        let (mut x, z) = (self.start[0], self.start[1]);
        let (mut yaw, mut height) = (east, self.height);
        let mut leg = LEGS[0].0;
        for (name, length) in LEGS {
            let spent = left.min(length);
            leg = name;
            match name {
                "pan" => x += speed * spent,
                "turn" => yaw += flight::TURN_RATE * spent,
                "climb" => height += (CLIMB_TO - self.height) * spent / length,
                _ => {}
            }
            if left < length {
                break;
            }
            left -= length;
        }
        if map_width > 0.0 {
            x = x.rem_euclid(map_width);
        }
        let lens = Lens { fov_y: flight::FOV_Y, near: 1.0, far: flight::FAR };
        let pose = Free::new(yaw, -orbit::LOWEST_ELEVATION, lens);
        (leg, Placement { center: [x, z], height, pose: Pose::Free(pose) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pan_wraps_east_on_the_map_and_a_bad_value_is_refused() {
        let flight = Flight::parse("5500,800,110").unwrap();
        let (_, wrapped) = flight.at(5.0, 5632.0);
        assert!((wrapped.center[0] - 108.0).abs() < 1e-2);
        assert!(Flight::parse("1,2,0").is_err());
        assert!(Flight::parse("1").is_err());
        assert!(Flight::parse("1,x").is_err());
    }
}
