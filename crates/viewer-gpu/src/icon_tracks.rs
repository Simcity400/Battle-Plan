#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Head {
    pub hour: i32,
    pub fraction: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IconFamily {
    Pins,
    Bases,
    Raids,
    Labels,
    Columns,
    Combats,
    Missions,
    Occupation,
    Intel,
    Resources,
    AdjacencyRules,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub hour: i32,
    pub at: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    places: std::sync::Arc<[Place]>,
    pub width: f32,
}

impl Track {
    pub fn new(places: &[Place], width: f32) -> Self {
        let mut held = places.to_vec();
        for at in (1..held.len()).rev() {
            held[at - 1].at[0] = unwrap_x(width, held[at].at[0], held[at - 1].at[0]);
        }
        Self { places: std::sync::Arc::from(held), width }
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    fn held(&self) -> &[Place] {
        &self.places
    }

    pub fn at(&self, head: Head) -> [f32; 3] {
        let held = self.held();
        let (Some(first), Some(last)) = (held.first(), held.last()) else {
            return [0.0; 3];
        };
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        let mut at = if over(first) <= 0.0 {
            first.at
        } else if over(last) >= 0.0 {
            last.at
        } else {
            let to = segment(held, head);
            let (from, to) = (&held[to - 1], &held[to]);
            let hours = (to.hour - from.hour) as f32;
            let into = if hours > 0.0 { (over(from) / hours).clamp(0.0, 1.0) } else { 1.0 };
            [0, 1, 2].map(|axis| from.at[axis] + (to.at[axis] - from.at[axis]) * into)
        };
        if self.width > 0.0 {
            at[0] = at[0].rem_euclid(self.width);
        }
        at
    }

    pub fn moves(&self) -> bool {
        let held = self.held();
        held.len() > 1 && held.iter().any(|place| place.at != held[0].at)
    }

    pub fn outside(&self, head: Head) -> i8 {
        let held = self.held();
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        match (held.first(), held.last()) {
            (Some(first), _) if over(first) < 0.0 => -1,
            (_, Some(last)) if over(last) > 0.0 => 1,
            _ => 0,
        }
    }

    pub fn held_still(&self, head: Head) -> bool {
        let held = self.held();
        let (Some(first), Some(last)) = (held.first(), held.last()) else {
            return true;
        };
        let over = |place: &Place| (head.hour - place.hour) as f32 + head.fraction;
        if over(first) <= 0.0 || over(last) >= 0.0 {
            return true;
        }
        let to = segment(held, head);
        held[to - 1].at == held[to].at
    }

    pub fn newest_hour(&self) -> i32 {
        self.held().last().map_or(0, |place| place.hour)
    }
}

fn segment(held: &[Place], head: Head) -> usize {
    held.iter()
        .position(|place| (head.hour - place.hour) as f32 + head.fraction < 0.0)
        .unwrap_or(held.len() - 1)
        .max(1)
}

pub fn unwrap_x(width: f32, reference: f32, x: f32) -> f32 {
    if width <= 0.0 || !reference.is_finite() || !x.is_finite() {
        return x;
    }
    x - width * ((x - reference) / width).round()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconAnchor {
    pub world: [f32; 3],
    pub cutoff: f32,
    pub family: IconFamily,
    pub moves: Option<u32>,
}

impl IconAnchor {
    pub fn key(&self) -> [u32; 6] {
        [
            self.world[0].to_bits(),
            self.world[1].to_bits(),
            self.world[2].to_bits(),
            self.cutoff.to_bits(),
            self.family as u32,
            self.moves.map_or(u32::MAX, |serial| serial),
        ]
    }

    pub fn moving(self, serial: Option<u32>) -> Self {
        Self { moves: serial, ..self }
    }

    pub fn at(&self, track: Option<&Track>, head: Head) -> [f32; 3] {
        track.map_or(self.world, |track| track.at(head))
    }
}

pub fn next_track() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests;
