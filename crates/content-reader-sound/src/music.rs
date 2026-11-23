use content_reader_key_names::key_name_products::KeyNamesExt as _;

use crate::{ResolvedLayers, defines, parser, sound::Notes, sound::registered};
use content_reader_script::native::{self, Shape};
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub path: String,
    pub volume: f32,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub track: usize,
    pub station: String,
    pub chance: Vec<parser::Item>,
    pub source: String,
}

#[derive(Clone, Debug, Default)]
pub struct Music {
    pub tracks: Vec<Track>,
    pub songs: Vec<Song>,
    pub notes: Notes,
    track_names: BTreeMap<Vec<u8>, usize>,
}

impl Music {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut music = Self::default();
        let assets = layers.descriptor_paths("music", &[".asset"])?;
        for file in defines::load_order("music/", ".asset", &assets) {
            let bytes = file.read_uncovered(layers)?;
            let path = file.text();
            match parser::parse(&bytes) {
                Ok(document) => music.declare_tracks(&document, &path, file.folder_text()),
                Err(error) => music
                    .notes
                    .note(format!("{path}: unreadable music asset ({error})")),
            }
        }
        let names = layers.key_names()?;
        let lists = layers.descriptor_paths("music", &[".txt"])?;
        for file in defines::load_order("music/", ".txt", &lists) {
            let bytes = file.read_uncovered(layers)?;
            music.read_songs(&bytes, &file.text(), &names);
        }
        Ok(music)
    }

    pub fn track(&self, name: &str) -> Option<usize> {
        self.track_names.get(name.as_bytes()).copied()
    }

    fn declare_tracks(
        &mut self,
        document: &parser::Document,
        path: &str,
        directory: Option<&str>,
    ) {
        for item in &document.items {
            if !item.keyword_is("music") {
                self.notes.note(format!(
                    "{path}: refused top-level {}",
                    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                ));
                continue;
            }
            let Some(fields) = item.value.items() else {
                self.notes.note(format!("{path}: refused scalar music"));
                continue;
            };
            for field in fields {
                let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
                let known = ["name", "file", "volume"]
                    .iter()
                    .any(|name| field.keyword_is(name));
                if !known {
                    self.notes
                        .note(format!("{path}: music refused field {key}"));
                } else if field.value.scalar_str().is_none() {
                    self.notes
                        .note(format!("{path}: music refused non-scalar {key}"));
                }
            }
            let text = |key: &str| {
                fields
                    .iter()
                    .rev()
                    .find(|field| field.keyword_is(key))
                    .and_then(|field| field.value.scalar_str())
                    .map(str::to_owned)
            };
            let (Some(name), Some(file)) = (text("name"), text("file")) else {
                self.notes
                    .note(format!("{path}: refused music without a name or file"));
                continue;
            };
            if self.track_names.contains_key(registered(&name)) {
                self.notes
                    .note(format!("{path}: refused repeated music {name}"));
                continue;
            }
            let Some(directory) = directory else {
                self.notes.note(format!(
                    "{path}: refused music {name}: the file's folder name isn't UTF-8, so its track path can't be named"
                ));
                continue;
            };
            let volume = match text("volume") {
                None => 1.0,
                Some(value) => {
                    match crate::crt::scan_f32(value.as_bytes()).filter(|value| value.is_finite()) {
                        Some(volume) => volume,
                        None => {
                            self.notes.note(format!(
                                "{path}: music {name} refused non-numeric volume {value}"
                            ));
                            1.0
                        }
                    }
                }
            };
            let file = file.replace('\\', "/");
            self.track_names
                .insert(registered(&name).to_vec(), self.tracks.len());
            self.tracks.push(Track {
                name,
                path: if directory.is_empty() {
                    file
                } else {
                    format!("{directory}/{file}")
                },
                volume,
                source: path.to_owned(),
            });
        }
    }

    fn read_songs(
        &mut self,
        bytes: &[u8],
        path: &str,
        names: &content_reader_script::key_names::KeyNames,
    ) {
        #[derive(Clone, Copy)]
        enum Node {
            File,
            Music,
            Chance,
            Unknown,
        }
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::File),
            Some(names),
            &mut |node, field| match node {
                Node::File => match field.keyword().as_deref() {
                    Some(b"music_station") => Shape::One,
                    Some(b"music") => Shape::Object(Node::Music),
                    _ => Shape::Skip,
                },
                Node::Music => match field.keyword().as_deref() {
                    Some(b"song") => Shape::One,
                    Some(b"chance") => Shape::Object(Node::Chance),
                    _ => Shape::ObjectIfOpen(Node::Unknown),
                },
                Node::Chance => native::mean_time_to_happen(field),
                Node::Unknown => Shape::Skip,
            },
        );
        if let Some(cause) = &read.unsettled {
            self.notes.note(format!(
                "{path}: songs after an unsettled read are unknown ({cause})"
            ));
        }
        for misread in &read.misreads {
            self.notes.note(format!("{path}: {misread}"));
        }
        let mut station: Option<String> = None;
        for item in read.items {
            if item.keyword_is("music_station") {
                station = item.value.scalar_str().map(str::to_owned);
                continue;
            }
            if !item.keyword_is("music") {
                self.notes.note(format!(
                    "{path}: refused top-level {}",
                    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                ));
                continue;
            }
            let fields = item.value.items().unwrap_or_default();
            for field in fields {
                if !field.keyword_is("song") && !field.keyword_is("chance") {
                    self.notes.note(format!(
                        "{path}: refused song-list field {}",
                        String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                    ));
                }
            }
            let Some(song) = fields
                .iter()
                .rev()
                .find(|field| field.keyword_is("song"))
                .and_then(|field| field.value.scalar_str())
                .map(str::to_owned)
            else {
                self.notes
                    .note(format!("{path}: refused music without a song"));
                continue;
            };
            let Some(track) = self.track(&song) else {
                self.notes
                    .note(format!("{path}: song {song} names no music track"));
                continue;
            };
            let Some(station) = station.clone() else {
                self.notes
                    .note(format!("{path}: song {song} precedes any music_station"));
                continue;
            };
            let chance = fields
                .iter()
                .rev()
                .find(|field| field.keyword_is("chance"))
                .and_then(|field| field.value.items())
                .map(<[_]>::to_vec)
                .unwrap_or_default();
            self.songs.push(Song {
                track,
                station,
                chance,
                source: path.to_owned(),
            });
        }
    }
}

pub fn enabled_stations(user: &Path) -> io::Result<Option<Vec<String>>> {
    enabled_stations_with(user, |path| std::fs::read(path))
}

pub fn enabled_stations_with(
    user: &Path,
    read: impl FnOnce(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<Option<Vec<String>>> {
    let bytes = match read(&user.join("playlists").join("settings.txt")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(parse_enabled(&bytes)?))
}

fn parse_enabled(bytes: &[u8]) -> io::Result<Vec<String>> {
    let document = parser::parse(bytes)?;
    Ok(document
        .items
        .iter()
        .filter(|item| item.keyword_is("enabled"))
        .flat_map(|item| item.value.items().unwrap_or_default())
        .filter(|item| item.key.is_none())
        .filter_map(|item| item.value.scalar_str())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_layers::test_support::Stack;

    #[test]
    fn songs_join_tracks_stations_and_chance_bodies_across_files() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_catalog");
        let (base, addon) = (stack.base(), stack.addon());
        stack.write(
            &base,
            "music/music.asset",
            r#"music = { name = "theme" file = "theme.ogg" volume = 0.65 }
               music = { name = theme file = "other.ogg" }"#,
        );
        stack.write(
            &base,
            "music/_songs.txt",
            r#"music_station = "base_music"
               music = { song = "theme" chance = { factor = 2 modifier = { factor = 0 has_war = yes } } }
               music = { song = missing tempo = fast }
               playlist_order = random"#,
        );
        stack.write(
            &addon,
            "music/pack/pack.asset",
            "music = { name = march file = \"m.ogg\" }
music = { name = quiet file = \"q.ogg\" volume = loud }",
        );
        stack.write(
            &addon,
            "music/pack/pack.txt",
            "music = { song = march }\nmusic_station = \"pack\"\nmusic = { song = march }",
        );
        let music = Music::read(&stack.layers(&[])).unwrap();
        assert_eq!(music.tracks.len(), 3);
        assert_eq!(music.tracks[0].path, "music/theme.ogg");
        assert_eq!(music.tracks[0].volume, 0.65);
        assert_eq!(music.tracks[1].path, "music/pack/m.ogg");
        let songs: Vec<_> = music
            .songs
            .iter()
            .map(|song| {
                (
                    music.tracks[song.track].name.as_str(),
                    song.station.as_str(),
                )
            })
            .collect();
        assert_eq!(songs, [("theme", "base_music"), ("march", "pack")]);
        let modifier = music.songs[0]
            .chance
            .iter()
            .find(|item| item.keyword_is("modifier"))
            .unwrap();
        let has_war = modifier
            .value
            .items()
            .unwrap()
            .iter()
            .find(|item| item.keyword_is("has_war"))
            .unwrap();
        assert_eq!(has_war.value.scalar(), Some(b"yes".as_slice()));
        let notes = music.notes.lines();
        for expected in [
            "music/music.asset: refused repeated music theme",
            "music/_songs.txt: song missing names no music track",
            "music/pack/pack.txt: song march precedes any music_station",
            "music/pack/pack.asset: music quiet refused non-numeric volume loud",
        ] {
            assert!(
                notes.contains(&expected.to_owned()),
                "{expected}: {notes:?}"
            );
        }
        for expected in [
            "music/_songs.txt: refused song-list field tempo",
            "music/_songs.txt: refused top-level playlist_order",
        ] {
            assert!(
                notes.contains(&expected.to_owned()),
                "{expected}: {notes:?}"
            );
        }
    }

    #[test]
    fn unmodelled_track_fields_are_refused_by_name() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_catalog_fields");
        let base = stack.base();
        stack.write(
            &base,
            "music/music.asset",
            r#"music = { name = a file = "a.ogg" tempo = fast volume = { 1 } }
               music = { name = b file = "b.ogg" fade = { in = 1 } }
               music = loud"#,
        );
        let music = Music::read(&stack.layers(&[])).unwrap();
        assert_eq!(music.tracks.len(), 2);
        let notes = music.notes.lines();
        for expected in [
            "music/music.asset: music refused field tempo",
            "music/music.asset: music refused non-scalar volume",
            "music/music.asset: music refused field fade",
            "music/music.asset: refused scalar music",
        ] {
            assert!(
                notes.contains(&expected.to_owned()),
                "{expected}: {notes:?}"
            );
        }
    }

    #[test]
    fn track_fields_in_any_case_load_without_a_refusal() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_catalog_case");
        let base = stack.base();
        stack.write(
            &base,
            "music/music.asset",
            r#"music = { NAME = t FILE = "t.ogg" VOLUME = 0.5 }"#,
        );
        let music = Music::read(&stack.layers(&[])).unwrap();
        assert_eq!(music.tracks.len(), 1);
        assert_eq!(music.tracks[0].volume, 0.5);
        let notes = music.notes.lines();
        assert!(
            !notes.iter().any(|note| note.contains("refused")),
            "{notes:?}"
        );
    }

    #[test]
    fn enabled_stations_read_the_playlist_settings() {
        assert_eq!(
            parse_enabled(b"enabled={\n\tbase_music\tsabaton }").unwrap(),
            ["base_music", "sabaton"]
        );
        assert_eq!(
            parse_enabled(br#"enabled={ "\"pack\"" pack }"#).unwrap(),
            ["\"pack\"", "pack"]
        );
        assert!(parse_enabled(b"").unwrap().is_empty());
    }

    #[test]
    fn escaped_quotes_stay_part_of_track_song_and_station_names() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_catalog_quotes");
        let base = stack.base();
        stack.write(
            &base,
            "music/music.asset",
            r#"music = { name = theme file = "plain.ogg" }
               music = { name = "\"theme\"" file = "quoted.ogg" }"#,
        );
        stack.write(
            &base,
            "music/songs.txt",
            r#"music_station = "\"pack\""
               music = { song = "\"theme\"" }
               music_station = pack
               music = { song = theme }"#,
        );
        let music = Music::read(&stack.layers(&[])).unwrap();
        assert_eq!(music.tracks.len(), 2, "{:?}", music.notes.lines());
        assert_eq!(
            music.tracks[music.track("\"theme\"").unwrap()].path,
            "music/quoted.ogg"
        );
        let songs: Vec<_> = music
            .songs
            .iter()
            .map(|song| {
                (
                    music.tracks[song.track].path.as_str(),
                    song.station.as_str(),
                )
            })
            .collect();
        assert_eq!(
            songs,
            [
                ("music/quoted.ogg", "\"pack\""),
                ("music/plain.ogg", "pack")
            ]
        );
    }

    #[test]
    fn a_track_name_is_added_and_found_by_its_first_63_bytes() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_catalog_long_names");
        let base = stack.base();
        let cut = "t".repeat(63);
        stack.write(
            &base,
            "music/music.asset",
            &format!(
                "music = {{ name = {cut}_one file = one.ogg }}
                 music = {{ name = {cut}_two file = two.ogg }}"
            ),
        );
        stack.write(
            &base,
            "music/songs.txt",
            &format!(
                "music_station = radio
                 music = {{ song = {cut} }}
                 music = {{ song = {cut}_one }}"
            ),
        );
        let music = Music::read(&stack.layers(&[])).unwrap();
        assert_eq!(music.tracks.len(), 1);
        assert_eq!(music.tracks[0].path, "music/one.ogg");
        assert_eq!(music.track(&cut), Some(0));
        assert_eq!(music.songs.len(), 1);
        assert_eq!(
            music.notes.lines(),
            [
                format!("music/music.asset: refused repeated music {cut}_two"),
                format!("music/songs.txt: song {cut}_one names no music track"),
            ]
        );
    }

    #[test]
    fn a_resolved_song_decodes_the_same_in_packets_and_as_one_track() {
        content_reader_script::test_game::enter();
        let stack = Stack::new("music_decode");
        let base = stack.base();
        stack.write(
            &base,
            "music/tone.asset",
            r#"music = { name = tone file = "tone.ogg" }"#,
        );
        stack.write(
            &base,
            "music/tone.txt",
            r#"music_station = "test_station" music = { song = tone }"#,
        );
        std::fs::write(
            base.join("music/tone.ogg"),
            include_bytes!("fixtures/synthetic-tone.ogg"),
        )
        .unwrap();
        let layers = stack.layers(&[]);
        let music = Music::read(&layers).unwrap();
        let [song] = music.songs.as_slice() else {
            panic!("expected one song, got {:?}", music.songs);
        };
        let track = &music.tracks[song.track];
        let bytes = layers.read_uncovered(&track.path).unwrap();
        let pcm = crate::ogg::decode(bytes.to_vec()).unwrap();
        assert_eq!((pcm.rate, pcm.channels), (8_000, 1));
        assert!(pcm.samples.iter().any(|sample| sample.abs() > 0.1));
        let mut stream = crate::ogg::open(bytes).unwrap();
        let mut streamed = Vec::new();
        while let Some(packet) = stream.decode_next().unwrap() {
            assert_eq!((packet.rate, packet.channels), (pcm.rate, pcm.channels));
            streamed.extend_from_slice(&packet.samples);
        }
        assert_eq!(streamed, pcm.samples);
    }
}
