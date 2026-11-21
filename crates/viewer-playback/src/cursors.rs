use eframe::egui::CustomCursorImage;
use std::io;

pub const GAME_LIST: [&str; 39] = [
    "normal.cur",
    "busy.ani",
    "cant_move.cur",
    "dragselect.ani",
    "selected.cur",
    "no_move.ani",
    "friendly_move.ani",
    "attack_move.ani",
    "embark.cur",
    "move_to_enemy_province.ani",
    "select_hint.ani",
    "add_units.cur",
    "build_buildings.ani",
    "brush.cur",
    "brush_detach.cur",
    "brush_detach_disabled.ico",
    "brush_front.cur",
    "brush_move.cur",
    "brush_delete.cur",
    "assign_air.ani",
    "chain_move.cur",
    "chain_move_naval.cur",
    "observe_country.cur",
    "assign_air.ani",
    "ping_offensive.cur",
    "ping_defensive.cur",
    "attach_air.cur",
    "operatives_build_intel_network.cur",
    "operatives_quiet_intel_network.cur",
    "operatives_counter_intelligence.cur",
    "operatives_root_out_resistance.cur",
    "operatives_boost_ideology.cur",
    "operatives_control_trade.cur",
    "operatives_diplomatic_pressure.cur",
    "operatives_propaganda.cur",
    "take_state_cursor.cur",
    "puppet_state_cursor.cur",
    "liberate_state_cursor.cur",
    "change_government_cursor.cur",
];

pub fn path(name: &str) -> Option<String> {
    GAME_LIST
        .iter()
        .find(|file| file.rsplit_once('.').is_some_and(|(stem, _)| stem == name))
        .map(|file| format!("gfx/cursors/{file}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapCursor {
    Normal,
    DragSelect,
    NoMove,
    AttackMove,
    AssignAir,
    PingOffensive,
    PingDefensive,
    FriendlyMove,
    MoveToEnemyProvince,
    AddUnits,
    ChainMove,
    ChainMoveNaval,
}

impl MapCursor {
    pub const ALL: [Self; 12] = [
        Self::Normal,
        Self::DragSelect,
        Self::NoMove,
        Self::AttackMove,
        Self::AssignAir,
        Self::PingOffensive,
        Self::PingDefensive,
        Self::FriendlyMove,
        Self::MoveToEnemyProvince,
        Self::AddUnits,
        Self::ChainMove,
        Self::ChainMoveNaval,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::DragSelect => "dragselect",
            Self::NoMove => "no_move",
            Self::AttackMove => "attack_move",
            Self::AssignAir => "assign_air",
            Self::PingOffensive => "ping_offensive",
            Self::PingDefensive => "ping_defensive",
            Self::FriendlyMove => "friendly_move",
            Self::MoveToEnemyProvince => "move_to_enemy_province",
            Self::AddUnits => "add_units",
            Self::ChainMove => "chain_move",
            Self::ChainMoveNaval => "chain_move_naval",
        }
    }
}

pub struct Animated {
    frames: Vec<CustomCursorImage>,
    steps: Vec<(usize, f64)>,
    period: f64,
}

const JIFFY: f64 = 1.0 / 60.0;

impl Animated {
    fn decode(path: &str, bytes: &[u8]) -> io::Result<Self> {
        let image = |cursor: installed_content_image::cursor::Cursor| CustomCursorImage {
            rgba: cursor.image.pixels.into(),
            size: [cursor.image.width as u16, cursor.image.height as u16],
            hotspot: cursor.hotspot,
        };
        let (frames, steps) = if path.ends_with(".ani") {
            let animated = installed_content_image::cursor::decode_ani(bytes)?;
            let steps = animated
                .steps
                .iter()
                .map(|&(frame, jiffies)| (frame as usize, f64::from(jiffies) * JIFFY))
                .collect();
            (animated.frames.into_iter().map(image).collect(), steps)
        } else {
            (vec![image(installed_content_image::cursor::decode(bytes)?)], vec![(0, f64::INFINITY)])
        };
        let period = steps.iter().map(|&(_, seconds)| seconds).sum();
        Ok(Self { frames, steps, period })
    }

    pub fn at(&self, seconds: f64) -> (&CustomCursorImage, Option<f64>) {
        if self.steps.len() == 1 {
            return (&self.frames[self.steps[0].0], None);
        }
        let mut into = seconds.max(0.0) % self.period;
        for &(frame, length) in &self.steps {
            if into < length {
                return (&self.frames[frame], Some(length - into));
            }
            into -= length;
        }
        let (frame, length) = self.steps[self.steps.len() - 1];
        (&self.frames[frame], Some(length))
    }
}

pub struct Cursors([Result<Animated, String>; MapCursor::ALL.len()]);

impl Cursors {
    pub fn load<B: AsRef<[u8]>>(read: impl Fn(&str) -> io::Result<B>) -> Self {
        Self(MapCursor::ALL.map(|cursor| Self::read(cursor.name(), &read)))
    }

    pub fn read<B: AsRef<[u8]>>(name: &str, read: impl Fn(&str) -> io::Result<B>) -> Result<Animated, String> {
        let path = path(name).ok_or_else(|| format!("{name}: not in the game's cursor list"))?;
        read(&path)
            .and_then(|bytes| Animated::decode(&path, bytes.as_ref()))
            .map_err(|error| format!("{path}: {error}"))
    }

    pub fn get(&self, cursor: MapCursor) -> Result<&Animated, &str> {
        self.0[cursor as usize].as_ref().map_err(String::as_str)
    }

    pub fn refusals(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|cursor| cursor.as_ref().err().map(String::as_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(red: u8) -> CustomCursorImage {
        CustomCursorImage { rgba: vec![red, 0, 0, 255].into(), size: [1, 1], hotspot: [0, 0] }
    }

    #[test]
    fn steps_follow_their_times_and_the_animation_repeats() {
        let animated = Animated {
            frames: vec![frame(1), frame(2)],
            steps: vec![(0, 0.1), (1, 0.3), (0, 0.2)],
            period: 0.6,
        };
        let shown = |seconds: f64| {
            let (image, next) = animated.at(seconds);
            (image.rgba[0], next.map(|next| (next * 1000.0).round() as i64))
        };
        assert_eq!(shown(0.0), (1, Some(100)));
        assert_eq!(shown(0.15), (2, Some(250)));
        assert_eq!(shown(0.45), (1, Some(150)));
        assert_eq!(shown(0.65), (1, Some(50)));
        let still = Animated { frames: vec![frame(9)], steps: vec![(0, f64::INFINITY)], period: f64::INFINITY };
        assert_eq!(still.at(12.0).0.rgba[0], 9);
        assert!(still.at(12.0).1.is_none());
    }

    #[test]
    fn a_cursor_that_does_not_read_is_refused_by_its_path() {
        let cursors = Cursors::load(|_: &str| Err::<Vec<u8>, _>(io::Error::other("absent")));
        assert_eq!(cursors.get(MapCursor::DragSelect).err(), Some("gfx/cursors/dragselect.ani: absent"));
        assert_eq!(cursors.refusals().count(), MapCursor::ALL.len());
    }

    #[test]
    fn every_cursor_is_named_by_the_games_list() {
        for cursor in MapCursor::ALL {
            assert!(path(cursor.name()).is_some(), "{cursor:?}");
        }
        assert_eq!(path("ping_offensive").as_deref(), Some("gfx/cursors/ping_offensive.cur"));
        assert_eq!(path("dragselect").as_deref(), Some("gfx/cursors/dragselect.ani"));
        let refused = Cursors::read("no_such_cursor", |_| Ok::<Vec<u8>, io::Error>(Vec::new())).err();
        assert!(refused.is_some_and(|why| why.contains("not in the game's cursor list")));
    }
}
