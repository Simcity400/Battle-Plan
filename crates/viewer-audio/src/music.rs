//! The music player: one track at a time from the enabled stations' songs.
//!
//! When no track plays, the player draws the next song as the game's music
//! player does: each enabled song weighs its chance for the viewing country
//! at the presented hour, held as the game's fixed point, times its place in
//! the recently-played history (the song played last weighs nothing, the
//! one before it 1/N of its chance, ... for `N =
//! NGame.MUSIC_PLAYER_RECENTLY_PLAYED_SIZE`, a song outside the history its
//! whole chance), at least 0.001 for a positive chance. The weights round to
//! integers in units of 1/100000 and the draw takes the player's own counter
//! hash. The drawn song joins the front of the history. A song whose chance
//! is zero or negative is not drawn. The next song starts as soon as the
//! track ends, with no gap or fade. Music keeps playing across seeks and
//! pauses, as the game's player does while its map changes underneath it,
//! and stops only when the view loses the audio.
//!
//! Tracks stream: the music loader decodes a track a packet at a time to
//! 16-bit samples as the game's Vorbis reader does and sends them on in
//! chunks ([`TrackStream`]), holding at most [`CHUNKS_AHEAD`] chunks ahead
//! of the player, so a track never sits decoded whole in memory. The game's
//! mixer plays the decoded samples as stereo frames at its own rate, so a
//! track at another rate plays at the mixer's rate and a mono track's
//! samples pair into frames, as they do in the game.

use crate::engine::Random;
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};

/// Interleaved samples per chunk a track's stream sends.
pub const CHUNK_SAMPLES: usize = crate::engine::RATE as usize / 2;
/// Chunks a stream decodes ahead of the player: about a second.
pub const CHUNKS_AHEAD: usize = 4;
/// A chance or weight unit: the game holds both as fixed point in 1/100000.
const FIXED_POINT: f32 = 100_000.0;
/// The least weight of a song with a positive chance.
const LEAST_WEIGHT: f32 = 0.001;
/// The weight of a song whose station is not enabled.
const DISABLED_WEIGHT: i64 = -100_000;

/// One decoded chunk: the track's 16-bit samples as decoded, or why the
/// stream stopped decoding.
pub type Chunk = Result<Vec<i16>, String>;

/// A track as it streams in from its loader; the stream ends when the
/// sender is dropped.
pub struct TrackStream {
    pub chunks: Receiver<Chunk>,
}

impl std::fmt::Debug for TrackStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrackStream")
    }
}

/// A bounded channel for one track's chunks: the sender blocks once
/// [`CHUNKS_AHEAD`] chunks wait, and fails once the player let go.
pub fn track_channel() -> (SyncSender<Chunk>, TrackStream) {
    let (sender, chunks) = mpsc::sync_channel(CHUNKS_AHEAD);
    (sender, TrackStream { chunks })
}

/// A decoded float sample as the game's Vorbis reader hands it to the
/// mixer: scaled by 32768, rounded to nearest and clamped to 16 bits.
fn to_s16(sample: f32) -> i16 {
    (sample * 32_768.0)
        .round_ties_even()
        .clamp(-32_768.0, 32_767.0) as i16
}

/// Converts and sends a track's packets until it ends, fails or the player
/// lets go of it; returns the packets read. `next` yields one packet at a
/// time (rate, channels, interleaved samples); the samples stay as decoded.
pub fn pump(
    mut next: impl FnMut() -> Result<Option<(u32, u16, Vec<f32>)>, String>,
    sender: &SyncSender<Chunk>,
) -> usize {
    let mut pending = Vec::with_capacity(CHUNK_SAMPLES);
    let mut read = 0;
    loop {
        match next() {
            Ok(Some((_, _, samples))) => {
                read += 1;
                pending.extend(samples.iter().map(|sample| to_s16(*sample)));
                if pending.len() >= CHUNK_SAMPLES {
                    let chunk = std::mem::replace(&mut pending, Vec::with_capacity(CHUNK_SAMPLES));
                    if sender.send(Ok(chunk)).is_err() {
                        return read;
                    }
                }
            }
            Ok(None) => {
                if !pending.is_empty() {
                    let _ = sender.send(Ok(pending));
                }
                return read;
            }
            Err(why) => {
                let _ = sender.send(Err(why));
                return read;
            }
        }
    }
}

/// Each song's chance for the viewing country at the presented hour, in
/// [`installed_content_sound::music::Music::songs`] order; `None` where the song's station
/// is not enabled. Refusals name chance conditions the recording cannot
/// answer; such a song keeps its base chance.
pub use map_display_data::Music as Chances;

#[derive(Debug)]
struct Playing {
    track: usize,
    stream: Option<TrackStream>,
    /// The chunk playing and the next sample in it.
    chunk: Vec<i16>,
    cursor: usize,
    /// The song's volume, the voice's volume in the mixer.
    volume: f32,
    /// The last callback's 16-bit gain step, `None` before the first.
    previous: Option<i32>,
    /// Whether any decoded samples arrived yet.
    started: bool,
}

#[derive(Debug)]
pub struct Player {
    playing: Option<Playing>,
    /// Songs drawn, the latest first, at most the recently-played size.
    history: VecDeque<usize>,
    /// The track [`Player::update`] asks the loader for.
    wanted: Option<usize>,
    /// The music player's own draws.
    random: Random,
    /// Songs drawn so far.
    draws: u64,
    /// Why a track stopped decoding, named once each for the notes.
    pub failures: Vec<String>,
}

/// The music player's seed; its draws are its own, apart from the mixer's.
const PLAYER_SEED: u32 = 0x4d55_5349;

impl Default for Player {
    fn default() -> Self {
        Self::new(PLAYER_SEED)
    }
}

/// A song's integer weight from its chance and its place in the history.
fn weight(chance: Option<f64>, place: Option<usize>, recent: usize) -> (i64, bool) {
    let Some(chance) = chance else {
        return (DISABLED_WEIGHT, false);
    };
    // The chance is fixed point in the game.
    let chance = (chance * f64::from(FIXED_POINT)).round() as i64 as f32 / FIXED_POINT;
    let weight = if chance > 0.0 {
        let factor = match place {
            Some(place) => (place as f32 / recent as f32).clamp(0.0, 1.0),
            None => 1.0,
        };
        (factor * chance).max(LEAST_WEIGHT)
    } else {
        0.0
    };
    let sign = if weight >= 0.0 { 1.0 } else { 0.0 };
    (((sign + weight * FIXED_POINT) - 0.5) as i64, weight > 0.0)
}

impl Player {
    pub fn new(seed: u32) -> Self {
        Self {
            playing: None,
            history: VecDeque::new(),
            wanted: None,
            random: Random::new(seed),
            draws: 0,
            failures: Vec::new(),
        }
    }

    /// Draws the next song when none plays. `recent` is the recently-played
    /// size the history keeps.
    pub fn update<'m>(
        &mut self,
        music: &'m installed_content_sound::music::Music,
        chances: Option<&Chances>,
        recent: usize,
    ) -> Option<&'m str> {
        if self.playing.is_some() {
            return None;
        }
        let chances = chances?;
        let mut any = false;
        let weights: Vec<i64> = (0..music.songs.len())
            .map(|song| {
                let place = self.history.iter().position(|played| *played == song);
                let (weight, positive) =
                    weight(chances.weights.get(song).copied().flatten(), place, recent);
                any |= positive;
                weight
            })
            .collect();
        if !any {
            return None;
        }
        let word = self.random.word();
        let song = crate::engine::weighted(&weights, word, None)?;
        self.draws += 1;
        self.history.push_front(song);
        self.history.truncate(recent);
        let track = music.songs[song].track;
        self.playing = Some(Playing {
            track,
            stream: None,
            chunk: Vec::new(),
            cursor: 0,
            volume: music.tracks[track].volume,
            previous: None,
            started: false,
        });
        self.wanted = Some(track);
        Some(&music.tracks[track].name)
    }

    pub fn take_wanted(&mut self) -> Option<usize> {
        self.wanted.take()
    }

    /// An opened track starts streaming; an unreadable one is skipped.
    pub fn provide(&mut self, track: usize, stream: Option<TrackStream>) {
        if let Some(playing) = &mut self.playing
            && playing.track == track
            && playing.stream.is_none()
        {
            match stream {
                Some(stream) => playing.stream = Some(stream),
                None => self.playing = None,
            }
        }
    }

    pub fn stop(&mut self) {
        self.playing = None;
        self.wanted = None;
    }

    /// Song indices are list-local, so history cannot survive a song-list change.
    pub fn forget_history(&mut self) {
        self.history.clear();
    }

    pub fn draws(&self) -> u64 {
        self.draws
    }

    /// Whether a drawn track waits for its stream or its first samples.
    pub fn waiting(&self) -> bool {
        self.playing
            .as_ref()
            .is_some_and(|playing| !playing.started)
    }

    /// Waits up to `limit` for the playing track's first chunk.
    pub fn prime(&mut self, limit: std::time::Duration) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(stream) = &playing.stream else {
            return;
        };
        if playing.started {
            return;
        }
        match stream.chunks.recv_timeout(limit) {
            Ok(Ok(chunk)) => {
                playing.chunk = chunk;
                playing.cursor = 0;
                playing.started = true;
            }
            Ok(Err(why)) => {
                if !self.failures.contains(&why) {
                    self.failures.push(why);
                }
                self.playing = None;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => self.playing = None,
        }
    }

    pub fn playing(&self) -> Option<usize> {
        self.playing.as_ref().map(|playing| playing.track)
    }

    /// The playing track's volume, while it streams.
    pub fn volume(&self) -> Option<f32> {
        self.playing
            .as_ref()
            .filter(|playing| playing.stream.is_some())
            .map(|playing| playing.volume)
    }

    /// Mixes one callback of the playing track into `buffer` (interleaved
    /// stereo 32-bit sums) at the 16-bit gain step `gain`, ramping from the
    /// last callback's step across the callback. A chunk not yet decoded
    /// leaves the rest of the callback silent; a track whose stream ended
    /// or failed ends.
    pub fn mix(&mut self, buffer: &mut [i32], gain: i32) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let Some(stream) = playing.stream.take() else {
            return;
        };
        let old = playing.previous.unwrap_or(gain);
        playing.previous = Some(gain);
        let frames = buffer.len() / 2;
        // `Some` once the track ends, with why it failed if it did.
        let mut end: Option<Option<String>> = None;
        'frames: for frame in 0..frames {
            let step = crate::engine::ramp(old, gain, frame, frames);
            for side in 0..2 {
                if playing.cursor >= playing.chunk.len() {
                    match stream.chunks.try_recv() {
                        Ok(Ok(chunk)) => {
                            playing.chunk = chunk;
                            playing.cursor = 0;
                            playing.started = true;
                            if playing.chunk.is_empty() {
                                break 'frames;
                            }
                        }
                        Ok(Err(why)) => {
                            end = Some(Some(why));
                            break 'frames;
                        }
                        // Not decoded yet: the callback plays on in silence.
                        Err(TryRecvError::Empty) => break 'frames,
                        Err(TryRecvError::Disconnected) => {
                            end = Some(None);
                            break 'frames;
                        }
                    }
                }
                buffer[frame * 2 + side] +=
                    crate::engine::scale(playing.chunk[playing.cursor], step);
                playing.cursor += 1;
            }
        }
        match end {
            None => playing.stream = Some(stream),
            Some(why) => {
                if let Some(why) = why
                    && !self.failures.contains(&why)
                {
                    self.failures.push(why);
                }
                self.playing = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
