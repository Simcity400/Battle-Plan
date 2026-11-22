use eframe::egui::CustomCursorImage;
use engine_executable::executable_tables::{Tables, cursors::CursorTable};
use std::io;

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

#[derive(Debug)]
pub struct Animated {
    frames: Vec<CustomCursorImage>,
    steps: Vec<(usize, f64)>,
    period: f64,
}

const JIFFY: f64 = 1.0 / 60.0;

impl Animated {
    fn decode(path: &str, bytes: &[u8]) -> io::Result<Self> {
        let image = |cursor: content_reader_image::cursor::Cursor| CustomCursorImage {
            rgba: cursor.image.pixels.into(),
            size: [cursor.image.width as u16, cursor.image.height as u16],
            hotspot: cursor.hotspot,
        };
        let extension = path.rsplit_once('.').map(|(_, extension)| extension);
        let (frames, steps) = if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("ani")) {
            let animated = content_reader_image::cursor::decode_ani(bytes)?;
            let steps = animated
                .steps
                .iter()
                .map(|&(frame, jiffies)| (frame as usize, f64::from(jiffies) * JIFFY))
                .collect();
            (animated.frames.into_iter().map(image).collect(), steps)
        } else if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("cur")) {
            (
                vec![image(content_reader_image::cursor::decode(bytes)?)],
                vec![(0, f64::INFINITY)],
            )
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported cursor file format",
            ));
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
    pub fn load_from_tables<B: AsRef<[u8]>>(
        tables: Option<Tables>,
        read: impl Fn(&str) -> io::Result<B>,
    ) -> Self {
        match tables {
            Some(tables) => match &tables.cursors {
                Ok(registry) => Self::load(Ok(registry), read),
                Err(withheld) => {
                    let why = withheld.to_string();
                    Self::load(Err(why.as_str()), read)
                }
            },
            None => Self::load(
                Err("map cursor registry withheld: the recording's executable tables are not loaded"),
                read,
            ),
        }
    }

    pub fn load<B: AsRef<[u8]>>(
        registry: Result<&CursorTable, &str>,
        read: impl Fn(&str) -> io::Result<B>,
    ) -> Self {
        Self(MapCursor::ALL.map(|cursor| Self::read(registry, cursor.name(), &read)))
    }

    pub fn read<B: AsRef<[u8]>>(
        registry: Result<&CursorTable, &str>,
        name: &str,
        read: impl Fn(&str) -> io::Result<B>,
    ) -> Result<Animated, String> {
        let registry = registry.map_err(|why| format!("{name}: {why}"))?;
        let path = registry.path(name)?;
        read(path)
            .and_then(|bytes| Animated::decode(path, bytes.as_ref()))
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

    fn cur_bytes(colour: [u8; 4], hotspot: [u16; 2]) -> Vec<u8> {
        let mut image = Vec::new();
        for value in [40u32, 1, 2] {
            image.extend_from_slice(&value.to_le_bytes());
        }
        image.extend_from_slice(&1u16.to_le_bytes());
        image.extend_from_slice(&32u16.to_le_bytes());
        image.extend_from_slice(&[0; 24]);
        image.extend_from_slice(&[colour[2], colour[1], colour[0], colour[3]]);
        image.extend_from_slice(&[0; 4]);
        let mut file = Vec::new();
        for value in [0u16, 2, 1] {
            file.extend_from_slice(&value.to_le_bytes());
        }
        file.extend_from_slice(&[1, 1, 0, 0]);
        file.extend_from_slice(&hotspot[0].to_le_bytes());
        file.extend_from_slice(&hotspot[1].to_le_bytes());
        file.extend_from_slice(&(image.len() as u32).to_le_bytes());
        file.extend_from_slice(&22u32.to_le_bytes());
        file.extend_from_slice(&image);
        file
    }

    fn ani_bytes(colours: &[[u8; 4]], jiffies: u32) -> Vec<u8> {
        let chunk = |id: &[u8; 4], body: &[u8]| -> Vec<u8> {
            let mut out = id.to_vec();
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
            if body.len() % 2 == 1 {
                out.push(0);
            }
            out
        };
        let words = |values: &[u32]| -> Vec<u8> {
            values.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
        let count = colours.len() as u32;
        let anih = chunk(b"anih", &words(&[36, count, count, 0, 0, 0, 0, jiffies, 1]));
        let mut fram = b"fram".to_vec();
        for &colour in colours {
            fram.extend_from_slice(&chunk(b"icon", &cur_bytes(colour, [1, 2])));
        }
        let list = chunk(b"LIST", &fram);
        let body = [anih, list].concat();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(b"ACON");
        out.extend_from_slice(&body);
        out
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
        let registry = CursorTable::from_paths([
            "gfx/cursors/dragselect.ani".to_owned(),
            "gfx/cursors/normal.cur".to_owned(),
        ])
        .unwrap();
        let cursors = Cursors::load(Ok(&registry), |_: &str| Err::<Vec<u8>, _>(io::Error::other("absent")));
        assert_eq!(cursors.get(MapCursor::DragSelect).err(), Some("gfx/cursors/dragselect.ani: absent"));
        assert_eq!(cursors.get(MapCursor::Normal).err(), Some("gfx/cursors/normal.cur: absent"));
        assert_eq!(
            cursors.get(MapCursor::PingOffensive).err(),
            Some("ping_offensive: not in the recorded cursor registry")
        );
        assert_eq!(cursors.refusals().count(), MapCursor::ALL.len());
    }

    #[test]
    fn two_recorded_registries_on_the_same_installation_resolve_distinct_bindings_and_withhold_unsupported_ones() {
        let root = std::env::temp_dir().join(format!("bp_cursors_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("gfx/cursors")).unwrap();
        std::fs::create_dir_all(root.join("gfx/alt")).unwrap();
        std::fs::write(
            root.join("gfx/cursors/normal.cur"),
            cur_bytes([255, 0, 0, 255], [1, 1]),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/cursors/dragselect.ani"),
            ani_bytes(&[[200, 10, 0, 255], [200, 20, 0, 255]], 6),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/cursors/assign_air.ani"),
            ani_bytes(&[[0, 180, 0, 255]], 6),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/alt/normal.cur"),
            cur_bytes([0, 0, 255, 255], [4, 5]),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/alt/dragselect.cur"),
            cur_bytes([0, 255, 0, 255], [2, 3]),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/alt/ping_defensive.ico"),
            cur_bytes([50, 50, 50, 255], [0, 0]),
        )
        .unwrap();

        let layers = content_reader_core::ResolvedLayers::from_folders(&[(root.as_path(), &[][..])])
            .unwrap();
        let read = |path: &str| layers.read_uncovered(path);

        let first_tables = synthetic_game::Executable::empty()
            .cursors([
                "gfx/cursors/normal.cur",
                "gfx/cursors/dragselect.ani",
                "gfx/cursors/assign_air.ani",
                "gfx/cursors/assign_air.ani",
            ])
            .build();
        let second_tables = synthetic_game::Executable::empty()
            .cursors([
                "gfx/alt/normal.cur",
                "gfx/alt/dragselect.cur",
                "gfx/cursors/attack_move.cur",
                "gfx/alt/attack_move.ani",
                "gfx/alt/ping_defensive.ico",
            ])
            .build();

        let first = Cursors::load_from_tables(Some(first_tables), read);
        let second = Cursors::load_from_tables(Some(second_tables), read);

        assert_eq!(first.get(MapCursor::Normal).unwrap().at(0.0).0.rgba.as_ref(), &[255, 0, 0, 255]);
        assert_eq!(first.get(MapCursor::Normal).unwrap().at(0.0).0.hotspot, [1, 1]);
        assert!(first.get(MapCursor::DragSelect).unwrap().at(0.0).1.is_some());
        assert_eq!(first.get(MapCursor::AssignAir).unwrap().at(0.0).0.rgba.as_ref(), &[0, 180, 0, 255]);
        assert!(first.get(MapCursor::PingOffensive).unwrap_err().contains("ping_offensive"));

        assert_eq!(second.get(MapCursor::Normal).unwrap().at(0.0).0.rgba.as_ref(), &[0, 0, 255, 255]);
        assert_eq!(second.get(MapCursor::Normal).unwrap().at(0.0).0.hotspot, [4, 5]);
        assert_eq!(second.get(MapCursor::DragSelect).unwrap().at(0.0).0.rgba.as_ref(), &[0, 255, 0, 255]);
        assert!(second.get(MapCursor::DragSelect).unwrap().at(0.0).1.is_none());
        assert!(second.get(MapCursor::AttackMove).unwrap_err().contains("attack_move"));
        assert!(second.get(MapCursor::AttackMove).unwrap_err().contains("multiple paths"));
        assert!(second.get(MapCursor::PingDefensive).unwrap_err().contains("gfx/alt/ping_defensive.ico"));
        assert!(second.get(MapCursor::PingDefensive).unwrap_err().contains("unsupported"));
        assert!(second.get(MapCursor::AssignAir).unwrap_err().contains("assign_air"));

        let withheld_tables = synthetic_game::Executable::empty()
            .withhold(synthetic_game::Part::Cursors, "not recorded in this recording")
            .build();
        let withheld = Cursors::load_from_tables(Some(withheld_tables), read);
        assert_eq!(withheld.refusals().count(), MapCursor::ALL.len());
        assert!(
            withheld
                .get(MapCursor::Normal)
                .unwrap_err()
                .contains("not recorded in this recording")
        );

        let absent = Cursors::load_from_tables(None, read);
        assert_eq!(absent.refusals().count(), MapCursor::ALL.len());
        assert!(
            absent
                .get(MapCursor::DragSelect)
                .unwrap_err()
                .contains("dragselect")
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
