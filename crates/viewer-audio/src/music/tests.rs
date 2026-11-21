use super::*;
use installed_content_sound::music::{Music, Song, Track};

fn music(songs: usize) -> Music {
    let mut music = Music::default();
    for index in 0..songs {
        music.tracks.push(Track {
            name: format!("t{index}"),
            path: format!("music/t{index}.ogg"),
            volume: 0.5,
            source: String::new(),
        });
        music.songs.push(Song {
            track: index,
            station: "base_music".into(),
            chance: Vec::new(),
            source: String::new(),
        });
    }
    music
}

fn chances(weights: Vec<Option<f64>>) -> Chances {
    Chances {
        weights,
        refusals: Vec::new(),
    }
}

#[test]
fn weights_follow_the_history_place_and_round_to_the_fixed_point() {
    assert_eq!(weight(Some(1.0), None, 10), (100_000, true));
    assert_eq!(weight(Some(1.0), Some(0), 10), (100, true));
    assert_eq!(weight(Some(2.0), Some(5), 10), (100_000, true));
    assert_eq!(weight(Some(0.0), None, 10), (0, false));
    assert_eq!(weight(Some(-3.0), None, 10), (0, false));
    assert_eq!(weight(None, None, 10), (DISABLED_WEIGHT, false));
    assert_eq!(weight(Some(0.000_001), None, 10), (0, false));
}

#[test]
fn draws_only_positive_chances_and_a_lone_song_plays_again() {
    let music = music(3);
    let chances = chances(vec![Some(1.0), Some(0.0), None]);
    let mut player = Player::new(3);
    assert_eq!(player.update(&music, Some(&chances), 10), Some("t0"));
    assert_eq!(player.take_wanted(), Some(0));
    assert_eq!(player.update(&music, Some(&chances), 10), None);
    let (sender, stream) = track_channel();
    sender.send(Ok(vec![16_384; 4])).unwrap();
    drop(sender);
    player.provide(0, Some(stream));
    let mut out = vec![0i32; 16];
    player.mix(&mut out, 8_192);
    assert_eq!(&out[..4], [8_192; 4]);
    assert_eq!(player.playing(), None, "a finished track ends");
    assert_eq!(player.update(&music, Some(&chances), 10), Some("t0"));
    assert_eq!(player.history, [0, 0]);
}

#[test]
fn the_history_keeps_the_recently_played_size() {
    let music = music(2);
    let chances = chances(vec![Some(1.0), Some(1.0)]);
    let mut player = Player::new(0);
    for _ in 0..5 {
        player.update(&music, Some(&chances), 3).unwrap();
        player.stop();
    }
    assert_eq!(player.history.len(), 3);
    let mut player = Player::new(0);
    player.update(&music, Some(&chances), 0).unwrap();
    assert!(player.history.is_empty());
    player.stop();
    assert_eq!(player.update(&music, None, 1), None);
}

#[test]
fn a_smaller_size_or_another_song_list_leaves_no_stale_history() {
    let music = music(4);
    let chances = chances(vec![Some(1.0); 4]);
    let mut player = Player::new(0);
    for _ in 0..4 {
        player.update(&music, Some(&chances), 4).unwrap();
        player.stop();
    }
    assert_eq!(player.history.len(), 4);
    player.update(&music, Some(&chances), 2).unwrap();
    player.stop();
    assert_eq!(player.history.len(), 2);
    player.forget_history();
    assert!(player.history.is_empty());
}

#[test]
fn a_streamed_track_plays_as_chunks_arrive() {
    let music = music(1);
    let chances = chances(vec![Some(1.0)]);
    let mut player = Player::new(1);
    player.update(&music, Some(&chances), 0);
    let (sender, stream) = track_channel();
    player.provide(0, Some(stream));
    let mut out = vec![0i32; 8];
    player.mix(&mut out, UNITY_STEP);
    assert!(out.iter().all(|value| *value == 0));
    assert_eq!(player.playing(), Some(0), "waiting for data is not the end");
    sender.send(Ok(vec![1_000, 2_000, 3_000])).unwrap();
    let mut out = vec![0i32; 8];
    player.mix(&mut out, UNITY_STEP);
    assert_eq!(out, [1_000, 2_000, 3_000, 0, 0, 0, 0, 0]);
    sender.send(Err("broken page".into())).unwrap();
    player.mix(&mut out, UNITY_STEP);
    assert_eq!(player.playing(), None);
    assert_eq!(player.failures, ["broken page"]);
}

const UNITY_STEP: i32 = 16_384;

#[test]
fn a_stream_decodes_only_ahead_of_the_player() {
    let (sender, stream) = track_channel();
    let reader = std::thread::spawn(move || {
        pump(|| Ok(Some((48_000, 1, vec![0.1; CHUNK_SAMPLES]))), &sender)
    });
    stream.chunks.recv().unwrap().unwrap();
    stream.chunks.recv().unwrap().unwrap();
    drop(stream);
    let read = reader.join().unwrap();
    assert!(read <= 2 + CHUNKS_AHEAD + 1, "{read} packets decoded");
}

#[test]
fn tracks_reach_the_mixer_as_decoded_16_bit_samples() {
    let (sender, stream) = track_channel();
    let mut packets = vec![(
        48_000u32,
        1u16,
        vec![0.5f32, -1.5, 1.0 / 65_536.0, 3.0 / 65_536.0],
    )];
    pump(|| Ok(packets.pop()), &sender);
    drop(sender);
    let chunk = stream.chunks.recv().unwrap().unwrap();
    assert_eq!(chunk, [16_384, -32_768, 0, 2]);
}
