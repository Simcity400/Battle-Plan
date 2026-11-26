
use crate::engine::Random;
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};

pub const CHUNK_SAMPLES: usize = crate::engine::RATE as usize / 2;
pub const CHUNKS_AHEAD: usize = 4;
const FIXED_POINT: f32 = 100_000.0;
const LEAST_WEIGHT: f32 = 0.001;
const DISABLED_WEIGHT: i64 = -100_000;

pub type Chunk = Result<Vec<i16>, String>;

pub struct TrackStream {
    pub chunks: Receiver<Chunk>,
}

impl std::fmt::Debug for TrackStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrackStream")
    }
}

pub fn track_channel() -> (SyncSender<Chunk>, TrackStream) {
    let (sender, chunks) = mpsc::sync_channel(CHUNKS_AHEAD);
    (sender, TrackStream { chunks })
}

pub(crate) fn decode_all(
    mut next: impl FnMut() -> Result<Option<(u32, u16, Vec<f32>)>, String>,
) -> TrackStream {
    let (sender, chunks) = mpsc::channel();
    let mut pending = Vec::with_capacity(CHUNK_SAMPLES);
    loop {
        match next() {
            Ok(Some((_, _, samples))) => {
                pending.extend(samples.iter().map(|sample| to_s16(*sample)));
                if pending.len() >= CHUNK_SAMPLES {
                    let chunk = std::mem::replace(&mut pending, Vec::with_capacity(CHUNK_SAMPLES));
                    let _ = sender.send(Ok(chunk));
                }
            }
            Ok(None) => {
                if !pending.is_empty() {
                    let _ = sender.send(Ok(pending));
                }
                break;
            }
            Err(why) => {
                let _ = sender.send(Err(why));
                break;
            }
        }
    }
    TrackStream { chunks }
}

fn to_s16(sample: f32) -> i16 {
    (sample * 32_768.0)
        .round_ties_even()
        .clamp(-32_768.0, 32_767.0) as i16
}

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

pub use map_display_data::Music as Chances;

#[derive(Debug)]
struct Playing {
    track: usize,
    stream: Option<TrackStream>,
    chunk: Vec<i16>,
    cursor: usize,
    volume: f32,
    previous: Option<i32>,
    started: bool,
}

#[derive(Debug)]
pub struct Player {
    playing: Option<Playing>,
    history: VecDeque<usize>,
    wanted: Option<usize>,
    random: Random,
    draws: u64,
    pub failures: Vec<String>,
}

const PLAYER_SEED: u32 = 0x4d55_5349;

impl Default for Player {
    fn default() -> Self {
        Self::new(PLAYER_SEED)
    }
}

fn weight(chance: Option<f64>, place: Option<usize>, recent: usize) -> (i64, bool) {
    let Some(chance) = chance else {
        return (DISABLED_WEIGHT, false);
    };
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

    pub fn update<'m>(
        &mut self,
        music: &'m content_reader_sound::music::Music,
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

    pub fn forget_history(&mut self) {
        self.history.clear();
    }

    pub fn draws(&self) -> u64 {
        self.draws
    }

    pub fn waiting(&self) -> bool {
        self.playing
            .as_ref()
            .is_some_and(|playing| !playing.started)
    }

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

    pub fn volume(&self) -> Option<f32> {
        self.playing
            .as_ref()
            .filter(|playing| playing.stream.is_some())
            .map(|playing| playing.volume)
    }

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
