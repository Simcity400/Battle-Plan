//! Music tracks, song lists and the user's enabled music stations.
//!
//! Every `.asset` file below `music/` declares tracks, `music = { name file
//! volume }`, its `file` relative to the asset's folder; the first track of a
//! name is kept. Every `.txt` file below `music/` is a song list: its
//! `music_station = "..."` names the station of the songs that follow it in
//! that file, and each `music = { song = ... chance = { ... } }` offers a
//! track with a weighted chance. The chance block is a mean-time body: a
//! `factor` base (one when absent) and `modifier` blocks whose triggers are
//! read with the load point's names and kept as data for evaluation against
//! the recording. Both kinds of file load in the recursive load order.
//!
//! The player chooses among the songs of the stations the user enabled in
//! `playlists/settings.txt` of the user folder.

use crate::{ResolvedLayers, defines, parser, sound::Notes};
use installed_content_core::parser::native::{self, Shape};
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
    /// Index into [`Music::tracks`].
    pub track: usize,
    pub station: String,
    /// The `chance` block's fields: a mean-time body.
    pub chance: Vec<parser::Item>,
    pub source: String,
}

#[derive(Clone, Debug, Default)]
pub struct Music {
    pub tracks: Vec<Track>,
    pub songs: Vec<Song>,
    pub notes: Notes,
    track_names: BTreeMap<String, usize>,
}

impl Music {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut music = Self::default();
        let assets = layers.descriptor_paths("music", &[".asset"])?;
        for path in defines::load_order("music/", ".asset", assets.iter().map(String::as_str)) {
            let bytes = layers.read_uncovered(&path)?;
            match parser::parse(&bytes) {
                Ok(document) => music.declare_tracks(&document, &path),
                Err(error) => music
                    .notes
                    .note(format!("{path}: unreadable music asset ({error})")),
            }
        }
        let names = layers.key_names()?;
        let lists = layers.descriptor_paths("music", &[".txt"])?;
        for path in defines::load_order("music/", ".txt", lists.iter().map(String::as_str)) {
            let bytes = layers.read_uncovered(&path)?;
            music.read_songs(&bytes, &path, &names);
        }
        Ok(music)
    }

    pub fn track(&self, name: &str) -> Option<usize> {
        self.track_names.get(name).copied()
    }

    fn declare_tracks(&mut self, document: &parser::Document, path: &str) {
        let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
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
            // Every field the reader does not model, or a modelled one that
            // is not a scalar, is refused by name and counted.
            for field in fields {
                let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
                let known = matches!(key.as_ref(), "name" | "file" | "volume");
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
                    .map(|value| value.trim_matches('"').to_owned())
            };
            let (Some(name), Some(file)) = (text("name"), text("file")) else {
                self.notes
                    .note(format!("{path}: refused music without a name or file"));
                continue;
            };
            if self.track_names.contains_key(&name) {
                self.notes
                    .note(format!("{path}: refused repeated music {name}"));
                continue;
            }
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
            self.track_names.insert(name.clone(), self.tracks.len());
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
        names: &installed_content_core::key_names::KeyNames,
    ) {
        #[derive(Clone, Copy)]
        enum Node {
            File,
            Music,
            Chance,
            /// An unknown song-list field's block, kept whole to be refused.
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
        // Fields the reader skips or refuses (unknown keys among them) are
        // named and counted.
        for misread in &read.misreads {
            self.notes.note(format!("{path}: {misread}"));
        }
        let mut station: Option<String> = None;
        for item in read.items {
            if item.keyword_is("music_station") {
                station = item
                    .value
                    .scalar_str()
                    .map(|value| value.trim_matches('"').to_owned());
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
                .map(|value| value.trim_matches('"').to_owned())
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

/// The music stations the user enabled (`playlists/settings.txt`'s
/// `enabled`), in file order; `None` when the file is absent.
pub fn enabled_stations(user: &Path) -> io::Result<Option<Vec<String>>> {
    let bytes = match std::fs::read(user.join("playlists").join("settings.txt")) {
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
        .map(|value| value.trim_matches('"').to_owned())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_core::test_support::Stack;

    #[test]
    fn songs_join_tracks_stations_and_chance_bodies_across_files() {
        engine_tables();
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
        assert!(
            music.songs[0]
                .chance
                .iter()
                .any(|item| item.keyword_is("modifier"))
        );
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
        // Fields the song-list reader does not know are named and counted.
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

    /// A field the track reader does not model, or a block where it reads a
    /// scalar, is refused by name and counted rather than dropped.
    #[test]
    fn unmodelled_track_fields_are_refused_by_name() {
        engine_tables();
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
    fn enabled_stations_read_the_playlist_settings() {
        assert_eq!(
            parse_enabled(b"enabled={\n\tbase_music\tsabaton }").unwrap(),
            ["base_music", "sabaton"]
        );
        assert!(parse_enabled(b"").unwrap().is_empty());
    }

    fn engine_tables() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    }
}

/// Reads the installation's song lists and decodes one track. Set `BATTLE_PLAN_GAME_DIR`.
#[cfg(test)]
#[test]
#[ignore = "reads the installation named by BATTLE_PLAN_GAME_DIR"]
fn installed_music_resolves_and_decodes() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let layers = ResolvedLayers::from_folders(&[(&game, &[])]).unwrap();
    let music = Music::read(&layers).unwrap();
    eprintln!("{} tracks, {} songs", music.tracks.len(), music.songs.len());
    for line in music.notes.lines() {
        eprintln!("  {line}");
    }
    let first = &music.tracks[music.songs[0].track];
    let pcm = crate::ogg::decode(layers.read_uncovered(&first.path).unwrap()).unwrap();
    eprintln!(
        "{}: {} Hz, {} channels, {} frames",
        first.path,
        pcm.rate,
        pcm.channels,
        pcm.frames()
    );
    assert!(pcm.frames() > 0);
    // Streamed packet by packet, the same track gives the same samples.
    let mut stream = crate::ogg::open(layers.read_uncovered(&first.path).unwrap()).unwrap();
    let mut streamed = Vec::new();
    while let Some(packet) = stream.decode_next().unwrap() {
        assert_eq!((packet.rate, packet.channels), (pcm.rate, pcm.channels));
        streamed.extend_from_slice(&packet.samples);
    }
    assert_eq!(streamed, pcm.samples);
}
